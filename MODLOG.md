# MODLOG

One entry per change: what, why, how it was verified. Record dead ends too.

## 2026-10-08 — M8c5: stars

**What:** decoded uSky's star field (`StarField`, the `StarsData`
catalogue in the game's resources, shader `Hidden/uSky/Stars`;
`docs/formats/sky.md` § Stars) and ported it: `sn-unity::parse_text_asset`,
`sn-assets::resource_bytes`, `stars.rs` (catalogue, unit test),
`stars.wgsl` + a pass in `sky_dome.rs` (after the fog, before the water).

**Verified (2026-10-08):** catalogue 218,640 bytes (9110 × 6 floats;
real-data test); 2460 stars kept; `out/m8c5-stars.png` (midnight: stars,
the planet). Tests, clippy, fmt: pass. **Not compared** with matched game
shots; ours look fainter than the user's (the game's bloom is not ported).

## 2026-10-08 — M8c5 (first pass): light shafts under water

**What:** decoded the game's `WaterSunShaftsOnCamera` (class, the scene's
values, both shader passes; `docs/formats/lighting.md` § Light shafts) and
ported it: `sun_shafts.rs` + `sun_shafts.wgsl` (half-resolution ray march
through the caustics projected along the sun, added after the water
surface). Also: `cast` is reserved in WGSL (again).

**Verified (2026-10-08):** `out/m8c5-shafts-sun.png`: faint vertical
shafts in the middle distance looking towards the sun; GPU 0.37 ms. Tests,
clippy, fmt: pass. **Not compared** with matched game screenshots; weaker
than the user's kelp shot, likely because the game's shafts are cut by the
sun's shadows (not ported yet, M8c6).

## 2026-10-08 — M8c4 (first pass): High quality waves (FFT)

**What:** the user plays with Water quality High (also bloom + lens dirt,
DoF, motion blur, AO, SSR, FXAA, dithering), so the waves are simulated, not
baked. Decoded `WaterDisplacementGenerator` (Phillips spectrum, the spectrum
update compute shader, the displacement packing; `docs/formats/water.md`
§ High quality waves). `sn-unity`: `FftWaves` (the generator's settings).
`sn-client`: `water_fft.rs` (initial spectrum, 2 unit tests), `water_fft.wgsl`
(spectrum update, our own FFT, packing), `--water-quality medium|high`
(default high). Plan for the remaining effects added to DESIGN (M8c5, M8d).

**Verified (2026-10-08):** scene values equal UnityPy's (wind 600 cm/s at
45°, choppy 1.3, amplitude 0.35, min wave 0.01; real-data test). Screenshot
`out/m8c4-high.png` next to the user's open-sea shot: finer, textured waves,
sparse foam. GPU: FFT 0.12 ms, surface 0.42 ms. Tests, clippy, fmt: pass.
**Hypothesis:** the FFT's sign (from the game's twiddle phase) and the
row/column orientation; a mirror would mirror the wind direction.

## 2026-10-08 — M8c3: game units, colour grading, Unity's ambient

**What:** after the user's screenshots (their game: High water quality)
showed our night far too bright:
- The game's post-processing found (`docs/formats/lighting.md` § Units):
  colour grading off by default (clamp, no tonemapping), bloom on. Our
  image now holds the game's values (one light unit = 1.0, was 2.55 from
  Bevy's sun lux and exposure) and `--color-grading off|neutral|aces`
  (default off) picks the tonemapper (neutral/aces: Bevy's nearest, not
  exact).
- Unity's own flat ambient (`CurrentSkyColor`, added by the G-buffer pass)
  now lit: it was missing, so surfaces were too dark.

**Verified (2026-10-08):** mean colours, ours vs the user's screenshots
(not the same places or times): night horizon (15, 21, 28) vs (5, 8, 16)
(was (31, 39, 51)); day sky (95, 142, 198) vs (111, 160, 222); underwater
fog (58, 115, 180) vs (63, 153, 221); sand (58, 113, 123) vs (108, 147, 120).
Tests, clippy, fmt: pass. **Needs matched screenshots** (same place and
time) to go further.

## 2026-10-08 — M8c3 (first pass): the game's lighting, caustics

**What:**
- Compared with the user's game screenshots (day, dusk and night, above and
  below; different places than ours). Sky and underwater fog are close;
  missing were caustics, the game's ambient and sunlight model, stars,
  object emission.
- Decoded the game's deferred lighting shader
  (`Hidden/Internal-DeferredShadingCustom`, directional cookie variant):
  `docs/formats/lighting.md`.
- `sn-unity`: `parse_resource_container` (`ResourceManager`);
  `WaterSurface.caustics_size`. `sn-assets`: `resource_texture`,
  `water_caustics` (64 frames). Real-data test extended.
- `sn-client`: `game_light.rs` + `game_light.wgsl`: the game's lighting
  (caustics, sunlight attenuated under water with the colour cast, top and
  bottom ambient, water glow as ambient, Blinn specular) replaces Bevy's PBR
  lighting in the terrain and object shaders. Terrain specular colour now
  used (SIG red or albedo red × spec colour). Per-frame values go through a
  small float texture rewritten on the GPU (no material updates).
- Sky fixes: bottom ambient (`colorOffset`'s ground branch); the cloud shade
  colour now includes the exposure (`colorOffset` applies it).

**Verified (2026-10-08, RTX 3080, 2400×1350):**
1. Caustics: 64 frames `WaterCaustics00…63`, 256² DXT1, 9 mips.
2. Screenshots `out/m8c3-start.png` (caustics on the sand, turquoise),
   `out/m8c3-below.png`, `out/m8c3-dusk-under.png` (green, orange glints, as
   the user's dusk shot), `out/m8c3-night-under.png` (dark navy).
3. `--benchmark 120 --gpu-timings` at the lifepod: mean 8.7 ms (was 11.0
   with Bevy's lighting), opaque pass 1.3 ms.
4. Workspace tests, real-data tests (5), clippy, fmt: pass.

**Not done:** per-pixel water settings (camera's used), sun shadows, object
specular/gloss/emission maps, light shafts, stars.

**Dead end:** `cast` is a reserved word in WGSL (shader failed to compile on
the first run).

## 2026-10-07 — M8c2: the game's sky (dome and sky map); sun direction fix

**What:**
- Decoded uSky's skybox and sky-map shaders and `uSkyManager`'s inputs;
  written up in `docs/formats/sky.md`.
- `sn-unity`: `SkyManager` now reads every field to the object's end
  (`SkyDome`: planet, clouds, night sky, moon, textures). `sn-assets`:
  `sky_textures`. Real-data test `sky_system_values` extended.
- `sn-client`: `sky_common.wgsl` (the sky function), `sky_dome.rs` +
  `sky_dome.wgsl` (sky map 256² with mips each frame; the dome where the
  depth buffer is empty, before the fog). The water surface reflects the sky
  map (its mean-colour stand-in is gone).
- **Fix (M8b):** the game's sky, day/night factors, water fog and water
  surface use `uSkyManager.SunDir`, a plain hour angle, not the directional
  light's direction (which always points down, moon-like at night). We used
  the light's: wrong by 7° at 09:36 and a day sky at night. Unit test
  `water_sun_turns_with_the_hour`. Also the fog clamp no longer renormalises
  (as the game).
- Fix: `Texture2D.color_space` doc comment had the meaning backwards (1 =
  sRGB colour, 0 = linear data; confirmed on terrain albedos vs normal maps).
  The code was right.
- Benchmarks no longer attach the free camera (mouse/keyboard input in the
  window had moved the camera during earlier screenshots — the likely cause
  of the odd M8c1 start shot).

**Verified (2026-10-07, RTX 3080, 2400×1350):**
1. `uSkyManager` read to its last byte; values equal UnityPy's.
2. Screenshots: `out/m8c2-above.png` (09:36, blue sky, clouds),
   `out/m8c2-up.png` (sun disc and halo), `out/m8c2-dusk.png` (20:30, orange
   clouds, glint path), `out/m8c2-night.png` (23:00, dark sky),
   `out/m8c2-below.png` (sky through the surface), `out/m8c2-start.png`
   (lifepod, unchanged). **Not compared with the game yet.**
3. `--benchmark 300 --gpu-timings` at the lifepod: sky 0.07 ms, water surface
   0.14 ms, mean 11.0 ms.
4. Workspace tests, real-data tests (5), clippy, fmt: pass.

**Dead end:** the first sky came out washed-out grey: our `beta_r()` helper
returns the game's `BetaR` × 1000 (the form the ambient colour code needs);
the sky shader needs `BetaR` itself.

## 2026-10-07 — M8c1: the water surface, ported from the game

**What:**
- Decoded the game's water surface: `WaterSurface` (class and scene values),
  its four shaders (surface, interpolate, normals, foam; render states from
  the shaders' parsed form), where the 64 baked wave frames and the foam
  textures are; written up in `docs/formats/water.md` § Water surface.
- `sn-unity`: `WaterSurface` reader, `AnimationCurve` (+ evaluate),
  `SkyManager` now reads on to `meanSkyColor`. 2 unit tests.
- `sn-assets`: `water_surface` (settings, frames in catalog order, foam
  textures), `Assets::catalog_object` (an object named by a catalog
  location; the prefab loader now uses it too). Real-data test
  `water_surface_values`, mean sky colour added to `sky_system_values`.
- `sn-client`: the fog model moved into `water_common.wgsl` (shared, same
  maths); `water_surface.rs` + `water_sim.wgsl` + `water_surface.wgsl`: the
  per-frame displacement / normal (mips) / foam maps and the surface, drawn
  after the fog pass over a copy of the fogged image. `--no-water-surface`.
  `--no-water-fog` now keeps the fog uniform (fog switched off in it). The
  benchmark logs the final camera position.

**Verified (2026-10-07, RTX 3080, 2400×1350):**
1. Our `WaterSurface` reading ends exactly at the object's last byte; values
   equal UnityPy's (patch 2000 cm, 5 s, gloss 400, curve 3.98 at 47 m, …);
   64 frames `WaterFrame00…63` in order, 256² RGBA32 linear; foam 1024²
   and 512² DXT1 sRGB. Displacement over all frames: x −65…72, y −71…75,
   z −67…68 cm.
2. `--benchmark 300 --gpu-timings` at the lifepod: wave maps 0.05 ms,
   surface (copy + draw) 0.55 ms, fog 0.08 ms; mean 10.8 ms.
3. Screenshots `out/m8c1-above.png` (camera at y = 4), `out/m8c1-below.png`
   (y = −6, looking up), `out/m8c1-start.png` (lifepod, unchanged: surface
   out of view); camera positions confirmed in the log. Near-white (foam)
   pixels: 1.9 % of the water from above, 2.8 % from below. **Not compared
   with the game yet.**
4. All tests (workspace, real-data `sn-assets`), clippy, fmt: pass.

**Not done / known:** no sky dome yet, so the reflected sky is the mean sky
colour and the sky above water is still our clear colour (M8c2); clip map
(cut-outs for bases/lifepod, shore foam) not read; "High" quality FFT waves
not ported. Our mesh differs from the game's adaptive patches.

**Dead end / unexplained:** one benchmark screenshot at the lifepod start
showed an above-water view; three later runs at the same start (camera
position logged) were right. Not reproduced; the window had a different
size in the odd runs (2400 vs 1600 wide). The uncommitted work of this
milestone was also lost once by a local repository reset and redone.

## 2026-10-07 — Build fix: debug `sn-client` failed to link (no code change)

**What happened:** `cargo build -p sn-client` (debug) failed with `LNK1120:
211 unresolved externals`, all Bevy generic instantiations with
`.llvm.<hash>` suffixes. Cause: stale/corrupt incremental artifacts in
`target/debug`, most likely from two cargo builds writing the same target
directory at once (a cargo build was waiting on the file lock during the
fix). Release builds were not affected.

**Fix:** `cargo clean -p sn-client`, then rebuild. Verified: debug and release
`--benchmark 30` both run (start area 1,393 batches; 89 / 95 fps).

**Not ours:** the Vulkan loader logs `ERROR … Failed to open JSON file …
EOSOverlayVkLayer-Win64.json` at start-up: a Vulkan layer registered by the
Epic Games overlay whose file is missing. Harmless.

## 2026-10-07 — M8b: the game's sky values (sun, ambient, sky fog) for the fog and the sun light

**What:**
- `sn-unity::sky`: Unity `Gradient` (read + evaluate), `SkyLight`
  (`uSkyLight`), `SkyManager` (`uSkyManager` up to the sky fog);
  `sn-assets::sky`. 2 unit tests, real-data test `sky_system_values`.
- `sn-client/sky.rs`: clock → sky timeline, sun path, day/night factors,
  sun colour × intensity, top ambient (with the class's Rayleigh colour
  offset), sky fog; `--time` (default 09:36, a new game's start). The fog
  now uses these instead of placeholders, and Bevy's sun takes the game's
  direction and colour (8000 lux per game light unit). 3 unit tests.
- How the layouts were found: UnityPy's type-tree generator on the game's
  assembly, once, in a throwaway venv in `out/` (`TypeTreeGeneratorAPI`).

**Verified (2026-10-07, RTX 3080):**
1. Our readers give exactly UnityPy's values (sun intensity 1.37, exposure
   0.66, sun direction −141°, gradient key times, …).
2. Our Rayleigh port reproduces the class's reference coefficients
   (5.81, 13.57, 33.13); noon sun points straight down, 06:00 is 25° up.
3. At 09:36: sky timeline 10.40 h, sun 73° up, colour (0.89, 0.73, 0.54),
   top ambient (0.042, 0.064, 0.098).
4. `--benchmark 120 --gpu-timings`: 15,276 objects, mean 9.4 ms; fog pass
   0.3 ms GPU. Screenshots `out/m8b-sky-overview.png`, `out/m8b-sky-closeup.png`
   (warmer sunlit sand). **Not compared with the game yet.**
5. Unit, real-data tests, clippy, fmt: pass.

**Dead ends:** UnityPy could not resolve the scene's scripts (they live in
another file); the objects were found with our own reader and decoded with
UnityPy by class name. The first attempt also read path id 390 from the
wrong serialized file of the bundle (204 bytes; the scene's is 716).

## 2026-10-07 — M8b (first pass): the game's underwater fog; plan for missing objects

**What:**
- Decoded the game's water fog (the compiled fog shader, its constant layout,
  the scene's `WaterscapeVolume` values, and how `WaterBiomeManager` fills its
  volume textures); written up in `docs/formats/water.md`.
- `sn-unity::WaterscapeVolume`, `sn-assets::water_volume`.
- `sn-client`: HDR camera with a readable depth buffer; `water.rs` (water at
  the camera from the biome map, batch overrides and the 145 biome settings,
  blended over 16 m cells, into a per-frame uniform) and a full-screen pass
  `water_fog.wgsl` before tone mapping, replacing Bevy's distance fog.
  `--fog-unit` (light calibration), `--no-water-fog` (HDR without the pass),
  `--gpu-timings` (GPU time per render pass at the end of a measurement).
- Roadmap: M7d spawn slots (the 90,289 `EntitySlotsPlaceholder`s; likely
  much of the missing small vegetation), M7e terrain grass, M7f scenes (the
  Aurora is the `aurora.unity` scene: 3,189 GameObjects; Lifepod 5 is in
  `escapepod.unity` and placed at run time) and skinned meshes; M8b next
  steps (sky values, atmosphere volumes, calibration against the game); M8c
  surface lighting, caustics, water surface, sky.

**Verified (2026-10-07, RTX 3080, game closed):**
1. `--benchmark 300 --gpu-timings` twice: identical object counts (15,276),
   0 warnings; mean 10.1 / 9.95 ms (≈ 100 fps; ≈ 8 ms before M8b), peak
   1.6 GiB; GPU time of the fog pass 0.062 / 0.067 ms (opaque pass 1.3 ms).
2. `--flythrough 1700 -80 0`: mean 4.62 ms (217 fps), worst 137 ms, 1.59 GiB.
3. Screenshots `out/m8b-overview.png`, `out/m8b-closeup.png`: turquoise
   near the seabed, blue into the distance, kelp silhouettes in the haze.
   **Not compared with the game yet** (sky values are placeholders).
4. Unit tests (1 new: coefficients), clippy, fmt; all real-data tests
   (incl. the scene's fog values) pass.

**Dead ends:** the first measurements (140–220 ms per frame, then two
`DeviceLost` crashes) were taken while `Subnautica.exe` and another
`sn-client` were running on the same GPU; with the GPU free, no crash and
the pass costs 0.06 ms. Several later runs sat at ≈ 16.6 ms regardless of
the shader (presentation pacing, not GPU work); `--gpu-timings` was added to
measure passes directly.

## 2026-10-07 — M8a: biomes and water settings, headless

**What:**
- `sn-world::biomes`: `BiomeMap` (`biomeMap.bin`: size from its last two
  bytes), `parse_biome_names` (`biomes.csv`), `BatchRootSettings` (a batch's
  override biome and fog from `LargeWorldBatchRoot`). 3 tests.
- `sn-unity::water`: `WaterBiomeManager` and `WaterSettings`.
- `sn-assets::water_biomes` (main scene); `main_scene` helper shared with the
  terrain materials. `sn-install::read_biome_map`.
- `sn-inspect biomes` and `biomes --at X Y Z`; real-data test
  `water_biomes_and_biome_map`.
- M8 split into M8a (data), M8b (underwater look), M8c (surface, caustics,
  sky) in `docs/DESIGN.md`; findings in `docs/formats/water.md`.
- How it was found: the game's classes read with `ilspycmd` into the
  gitignored `out/decompiled/` (same tool as for `Voxeland` earlier); nothing
  copied.

**Verified (2026-10-07):**
1. `sn-inspect biomes`: 145 biomes with plausible values (e.g. Safe Shallows
   absorption 125/20/4, 28 °C; lava 80 °C); map 1024 × 1024, 4 voxels per
   cell, 16 indices used; 467 batches override their biome. Biomes without
   settings: only `void` (open ocean) and `EmperorFacility` (the game falls
   back to its defaults for those).
2. `biomes --at`: lifepod (0, −10, 0) → `safeShallows`; (1700, −80, 0) →
   `crashZone`. Other places not checked against the game.
3. Unit, real-data tests, clippy, fmt: pass.

## 2026-10-07 — M7c: world objects in the client

**What:**
- `sn-client/objects.rs`: a worker thread (own asset index, catalog,
  `prefabs.db`) reads a batch's baked cells, loads each placed prefab once
  and sends new textures, materials, meshes (one per sub-mesh, only the
  vertices it uses; z flipped, winding reversed, tangent w negated) and the
  batch's instances. The main thread uploads assets once and spawns one
  entity per placed mesh part. Cell level *n* is shown while its batch's
  terrain level of detail is ≤ *n* (our choice). The worker forgets loaded
  bundles beyond 512 MiB (`Assets::trim_cache`).
- `object_look.rs` + `object.wgsl`: standard material with `_MainTex` ×
  `_Color` (texture scale/offset), alpha clip (`MARMO_ALPHA_CLIP`,
  `_Cutoff`), blending (render queue ≥ 2500, `MARMO_ALPHA`, `WBOIT`,
  `_ALPHAPREMULTIPLY_ON`), two-sided when `_MyCullVariable` = 0, and the
  game's DXT5nm normal maps through Bevy's mikktspace helpers.
- Shared texture upload (`textures.rs`, moved out of `terrain_look.rs`).
- `Prefab::visible_nodes` falls back to the most detailed LOD level with a
  plain mesh (LOD 0 is sometimes only skinned, e.g. `BrainCoral`); nodes
  record `skinned`. `sn-inspect prefab --placed` reports skinned prefabs.
- Not drawn: creatures (`WorldEntities/Creatures/…`: they move and animate;
  their spawn points were drawn frozen, e.g. Reefbacks hanging in the water),
  skinned meshes, extra (multi-pass) materials, LOD levels > the best one.
- `--no-objects`; stats log objects per cell level; benchmark/flythrough wait
  for objects as well as terrain.
- `sn-client` now depends on `sn-unity` (reads object materials).

**Verified (2026-10-07, RTX 3080, 1600×900):**
1. `--benchmark 120` twice: 15,276 entities (cell levels 0–3: 4,670 / 4,333 /
   1,582 / 4,691) from 775 prefabs, 0 warnings, identical in both runs;
   mean 8.27 / 8.06 ms (121 / 124 fps), worst 18.9 ms; peak memory 1.57 GiB
   (terrain only: 6.1 ms, 1.16 GiB).
2. `--flythrough 1700 -80 0`: mean 3.94 ms (254 fps), p95 6.4 ms, worst 136 ms
   (spikes while assets upload; not investigated), peak 1.59 GiB; objects
   despawn and respawn per level as the camera moves.
3. Screenshots (`out/m7c-overview.png`, `out/m7c-closeup.png`): coral and
   plants sit on the matching purple/orange terrain patches, kelp and a wreck
   piece in place; the floating boulder in the overview is
   `FloatingStone4_Floaters` (the game's floating rocks, minus the Floater
   creatures). **Not compared with the game side by side.**
4. `cargo test --workspace` (2 new tests: mirrored transforms, level
   mapping), clippy, fmt, real-data tests: pass.

**Dead ends:** first run peaked at 4.31 GiB: the worker's bundle cache kept
every decompressed bundle, and every sub-mesh copied its mesh's full vertex
arrays. Cache cap + per-sub-mesh vertices: 1.6 GiB. The double-sided flag
bit in Bevy is `1 << 4`, not `2`; caught before running.

## 2026-10-07 — M7b: prefabs → meshes

**What:**
- `sn-unity`: `Catalog` (Addressables catalog: own JSON + base64 readers),
  `Mesh` + `MeshGeometry` (interleaved vertex streams with every vertex
  format, and Unity's compressed meshes), `MeshFilter`, `MeshRenderer`,
  `GameObject`, `TransformNode`, `LodGroup`, `AssetBundleManifest`.
  7 unit tests (incl. a corrupted-catalog test).
- `sn-assets`: `Assets::catalog`, `Assets::prefab` (key → bundle →
  container → hierarchy with meshes, materials, LOD levels, active flags),
  `Assets::mesh` (inline or `.resS` vertex data).
- Fixed on the way: externals named `Library/…` (capital L) didn't resolve
  to Unity's built-in resources; `Assets::standalone` keyed files by their
  relative path, so `FileRef::file` could panic; `resource_range` could
  overflow on a bad offset.
- `sn-inspect prefab <KEY>` (hierarchy, OBJ export to `out/prefabs/`) and
  `prefab --placed [--oracle]`.
- Opt-in real-data test `prefabs_resolve_and_meshes_decode` in `sn-assets`.
- Format notes: `docs/formats/unity.md` (prefab/mesh classes, prefabs,
  Addressables). No new dependencies.

**Verified (2026-10-07):**
1. `sn-inspect prefab --placed`: all 1,369 placed prefabs load (324 with LOD
   groups, 457 draw nothing); 3,263 distinct meshes, 16.6 M vertices, 13.9 M
   triangles; 0 errors; 5.4 s.
2. Oracle: the same 3,263 meshes decoded by UnityPy (throwaway script in
   `out/`) agree on name, vertex count and triangles per sub-mesh; position
   sums agree to float rounding (max 1.29 on sums up to 2.6 × 10⁷).
3. OBJ exports have plausible sizes (coral 0.4–3 m, Aurora hallway
   44 × 19 × 58 m). Opened in Blender by the user (2026-10-07): the models
   came in far from the origin (export bug, fixed, see below); not re-checked
   in Blender after the fix — bounds now start at the pivot.
4. `cargo test --workspace`, clippy, fmt: pass; real-data tests pass.

**Dead ends / surprises:** the first export of `Spiral_blue_thing_cluster_07`
was 10 km wide: its root has scale 0.0001 and the export left the root out.
The placements carry that scale too, which confirms that a placement
replaces the root transform; stand-alone exports now apply the root's
rotation and scale. The user's Blender check then showed models far from
the origin: the export had also applied the root's *position* (where the
prefab sat in the artist's scene, e.g. −702, −105, −772). Left out now;
pivots sit at the origin (corals rest on y ≈ 0). 52
meshes are compressed (first run: 52 errors), so the compressed form had to
be decoded rather than skipped.

## 2026-10-06 — M7a: world object placements, headless

**What:**
- `sn-world`: `wire` (minimal protobuf wire reader, checked, byte offsets in
  errors) and `entities`: `ObjectTree` (batch objects), `BatchCells` (baked
  cells), `parse_prefab_database`, world transforms through the parent chain.
  `BatchCoord::from_file_name` (octree names now use it too). 6 + 3 new tests,
  including every truncation and byte flip of a sample.
- `sn-install`: `read_batch_objects`, `read_batch_cells`,
  `read_prefab_database`, `object_batches`, `cell_batches`; directory scans
  share one helper.
- `sn-inspect entities <X> <Y> <Z>` and `entities --all`.
- Opt-in real-data test `crates/sn-install/tests/entities.rs`.
- Format notes: `docs/formats/entities.md`.
No new dependencies: the wire reader is ~150 lines of our own.

**How the format was found:** throwaway Python dumpers in `out/` (not
committed) tried a grammar on all files before any Rust was written.

**Verified (2026-10-06):**
1. `sn-inspect entities --all`: 2,975 batch-object files (5,779 objects) and
   1,606 cell files (437,003 cells, 414,067 objects), 0 errors, 0 unresolved
   ClassIds, 0 missing parents, about 1.0 s. Two runs print the same hash
   (`660cae260060d787`).
2. Positions: 63 batch roots are stored off their batch corner (moved back;
   hypothesis in the format doc), 1,473 cell objects sit at exactly the
   world origin, 32 others are outside their batch (30 within 13 m). Cell
   roots lie on a 16 m (level 0) / 32 m (levels 1–3) grid.
3. `cargo test --workspace`, clippy, fmt: pass. Real-data test passes
   (`cargo test --release -p sn-install -- --ignored`).

**Dead ends:** the first prototype read one cells file and took header field
2 (256) for a constant; across all files it is the cell count, and the
version is 9 or 10. Caught by the real-data test, which asserted version 9.

## 2026-10-06 — M6b: the game's per-level cap on material layers

**Why:** the redone M6b drew every type of every chunk at every distance:
~8,400 blended draws in the start area, ~9 ms per frame. The game doesn't.
`StreamingAssets/SNUnmanagedData/clipmaps-{high,medium,low}.json` (the
terrain clipmap settings) cap the types per chunk per level (`maxBlockTypes`,
"high": 32, 8, 2, 1, 1), and the chunk mesher keeps the most-used types
(by face count) before sorting by layer. The files also confirm
`chunkMeshRes` 16 and the 9-vertex mesh at level 0 only.

**What:** `LayerSettings::max_types`; the client caps our levels 0–3 at
32, 2, 1, 1 (matched to the game's levels by sample spacing; that mapping is
ours). One new test in `sn-mesh` (8 there now).

**Verified (2026-10-06, RTX 3080):**
1. `cargo test --workspace`, clippy, fmt: pass.
2. `sn-client --benchmark 120`: start area 1,393 batches, 2,050 meshes (was 10,076),
   7.8 M triangles; mean 6.06 ms (165 fps, was 12.4 ms), p95 6.5 ms.
3. `--flythrough 1700 -80 0`, three runs: mean 15.4, 3.2 and 3.8 ms. The first
   run is an outlier (the GPU sat at 225 MHz idle clocks; not investigated
   further); the other two give 315 / 260 fps. Peak memory 1.16 GiB.
4. Visual: near terrain unchanged (`out/client-benchmark.png` vs
   `out/client-benchmark-uncapped.png`); far chunks show only their dominant
   materials. **Not compared against the real game.**

## 2026-10-06 — M6b redone: the game's own material layering and terrain shader

**Why:** the first M6b (entry below) didn't match the game, and broke docs.
- It painted the batch's "base" material (lowest layer, ties by triangle count)
  under the *whole batch*, then dithered the other materials over it. Where an
  overlay faded, a material from elsewhere in the batch showed through: the
  random green patches on sand in its screenshot.
- The Laplacian smoothing and Bayer-dither `discard` were invented. The game
  uses real alpha blending, and none of the material's border settings were
  used.
- Its MODLOG/README claims ("completely eliminated", 300+ fps with blending)
  were not backed by the screenshot. It also deleted the README's controls,
  licence and credits sections, rewrote the DESIGN tree glyphs, put a literal
  `\n\n` into `terrain-materials.md`, and added a stray space to the M4 entry.
  All restored.

M6 mistakes, also fixed:
- The cap texture was used for every face with a large |normal.y|, so
  overhangs and cave ceilings got sand. The game uses the cap only on upward
  slopes, with a ragged switch to the side texture (`_CapBorderBlend*` + cap
  alpha).
- Projections were in Bevy space with other axes: X (z,y) / Z (x,y) instead
  of the game's X (y,z) / Z (y,x) in Unity space. The textures were mirrored
  and rotated against the game.
- The triplanar exponent was a fixed 4, not each material's `_TriplanarBlendRange`.
- Material colours were used as stored (sRGB). The game is linear-colour-space
  (PlayerSettings, read with UnityPy), so they are converted.
- Doc claimed both kinds share one shader and listed 133 plain / 100 cap types.
  Really there are 3 shaders and 114 plain / 119 cap/side types.

**What:**
- Found how the game does it (documented in `docs/formats/terrain-materials.md`):
  property values of all materials (new `sn-inspect terrain-materials --props`),
  the terrain shaders' Direct3D bytecode (LZ4 blob in the `Shader` object,
  disassembled with Windows' `d3dcompiler_47.dll`, by throwaway scripts in
  `out/`), the scene's `Voxeland.chunkSize` (16), and the chunk mesher in
  `Assembly-CSharp.dll` (read for behaviour only).
- `sn-mesh::build_layers` (new, pure, 7 tests): per-chunk layers ordered by
  (`layer`, type id), weights = share of adjacent faces, 9-vertex faces at
  level of detail 0.
- `sn-assets`: materials carry blend settings, SIG maps, specular colours and
  emission scales.
- `sn-client`: rank 0 opaque, later ranks alpha-blended in order (shared
  bounding box per batch plus a depth bias = rank). `terrain.wgsl` is a port of
  the game's shader: cap/side by slope, border alpha from weight + splotch,
  border tint, SIG emission, gloss → roughness. 4 new tests. Stats log the
  number of meshes on screen.

**Verified (2026-10-06, RTX 3080, 1600×900):**
1. `cargo test --workspace`, `cargo clippy --workspace --all-targets`: pass, no warnings.
2. `sn-inspect terrain-materials`: result OK, 211 textures (183 + 28 SIG maps).
   Real-data test `sn-assets` (`-- --ignored`) updated to 211 textures, 119 cap/side types; passes.
3. `sn-client --benchmark 120`: start area 1,393 batches, 10,076 meshes,
   9.7 M triangles; mean 12.4 ms (81 fps), p95 15.6 ms. With the blended layers
   skipped (temporary experiment): 1,653 meshes, 3.3 ms. So the ~8,400 blended
   draws cost ~9 ms. Without face subdivision: 5.4 M triangles but no faster
   (14.4 ms), so the cost is draw calls, not triangles.
4. `--flythrough 1700 -80 0`: mean 5.1 ms (198 fps), p95 16.8 ms, worst 82 ms,
   peak memory 1.29 GiB. LOD-0 meshing mean 427 ms (was ~400 ms).
5. Visual (`out/client-benchmark.png` vs `out/client-benchmark-m6b.png`): no
   foreign patches, coral/grass borders are ragged blobs instead of voxel
   steps, and cliff tops are sand turning into rock by slope. The game itself
   was not run side by side: **not compared against the real game.**

**Not done:** specular colour (the game's deferred lighting is custom; unchecked),
lava flow animation, the game's lighting/fog (M8), and fewer draw calls
(merge layers across batches, or one bindless/texture-array material).

**Dead ends:** the first M6b's screen-space dither (below) was dropped in
favour of real alpha blending. Reading the shader's binding table, our first
parse got the offsets wrong: entries are 4-byte aligned from the start of the
blob, not from the start of the file.

## 2026-10-06 — M6b: soft blending between terrain materials (superseded; see above)

**What:**
- `sn-client`: soft blending between adjacent terrain materials, eliminating blocky voxel borders.
- Material layer hierarchy: terrain materials in Subnautica specify `VoxelandBlockType.layer` (e.g.
  `Sand02ToCoral15` = -100, `SS_SandToRock` = -99, `Sand02` = 0). Lower layers serve as the base
  strata foundation; higher layers are overlays.
- Mesh partitioning (`split_by_material`): builds an adjacency graph, computes normalized per-vertex
  material shares, and performs 1 step of Laplacian smoothing. The base material covers the entire
  batch (uvs = [1.0, 0.0], 100% solid, prevents cracks/holes). Overlay materials extend across
  boundaries and carry smoothed blend weights in vertex UVs (`Mesh::ATTRIBUTE_UV_0.x`).
- Shader (`terrain.wgsl`): added a 4×4 Bayer screen-space ordered dither discard when `blend_w < 0.999`.
  Fragments with `smoothstep(0.0, 1.0, blend_w)` below the Bayer threshold are discarded, revealing the
  underlying base material beneath. Discard avoids transparency sorting artifacts and z-fighting,
  keeping full depth buffer correctness.
- `sn-inspect materials`: displays the `layer` column in material listings and region census.
- Unit tests: `test_split_by_material_single_material` and `test_split_by_material_blending` in `sn-client`.

**Verified (2026-10-06):**
1. Benchmark on real Subnautica assets (`cargo run -p sn-client --release -- --benchmark 60`):
   Start area loaded in 1.72 s (1,393 batches, 6,967,128 triangles).
   60 frames rendered at mean 3.05 ms (328 fps), p95 3.64 ms, worst 4.03 ms. Peak memory 0.96 GiB.
2. Visual comparison: `out/client-benchmark.png` vs pre-M6b `out/client-benchmark-before-m6b.png`.
   Blocky voxel staircases at material boundaries are completely eliminated; sand, rock, and coral
   blend naturally and smoothly.
3. Tests & clippy: 40 tests passed across workspace (including blending unit tests). Clippy clean.

## 2026-10-06 — M6: textures and real terrain materials

**What:**
- `sn-unity`: `Texture2D` (2019.4 layout) with `decode_rgba` (DXT1/5, BC4/5/7, RGBA32, RGB24, Alpha8, …);
  `PPtr`, `Material`, `MonoScript`, `MonoBehaviourHeader`; game scripts `Voxeland`, `VoxelandBlockType`,
  `VoxelandBlockTypePrefab`; `BundleDirectory` (header + directory only, from a file prefix).
- New dependency: `texture2ddecoder` 0.1.2 (MIT OR Apache-2.0, pure Rust): block-compressed texture decoding.
- New crate `sn-assets` (layer 3): bundle index from directory prefixes (5,467 bundles in ~0.5 s), cached
  loading of bundles *and* standalone files (`resources.assets`; `.resS` read by byte range), cross-file
  `PPtr` resolution (`archive:/CAB-…`, player files, `library/` → `Resources/`), `terrain_materials`.
- `sn-install`: `read_file_prefix`, `read_file_range`.
- `sn-inspect`: `textures [--pixels]`, `textures --census`, `terrain-materials`.
- `sn-client`: real terrain look. DXT textures uploaded compressed with mip chains, `terrain.wgsl` triplanar
  shader on top of Bevy's standard material (cap/side layers, DXT5nm normal maps, whiteout blend), built-in
  shader via `embedded_asset!`; `--debug-colours` keeps the old false colours.
- `docs/formats/terrain-materials.md`; Texture2D/Material layouts in `docs/formats/unity.md`.

**Findings:** texture formats are DXT5/DXT1 (+ a few RGBA32/Alpha8/RGB24, one BC7), **no crunch**. Type ids map to
block types from the scene's `Voxeland.types` (56) and `BlockPrefabs` prefabs keyed by `globalId` (233, plus 10 with
id 0, skipped). The layouts came from the game's own DLLs via UnityPy's type-tree generator (dev machine only).
Terrain materials are plain (`_MainTex`…) or cap/side blends (`_CapTexture`/`_SideTexture`…); normal maps are DXT5nm.

**Verified (2026-10-06):**
1. Texture metadata identical to UnityPy for 1,062 textures in 43 files; decoded pixel CRCs identical for 506
   textures (all formats in the sample). The first run differed only for Alpha8: I had guessed (255,255,255,a);
   UnityPy and the GPU give (0,0,0,a). Fixed.
2. `sn-inspect terrain-materials`: every type id used by the octrees (209 non-empty) has a material with cap and
   side textures; 183 textures, read in ~0.2 s.
3. Real-data test `sn-assets`: 233 materials, all textures decode completely, scales sane. Workspace: 47 tests,
   clippy clean.
4. Client: flythrough to the crater edge 374 fps mean, worst frame 45 ms, peak memory 0.90 GiB, 138 MiB of terrain
   textures on the GPU. Screenshots (gitignored) show sand with ripples, porous and striated rock, moss.

**Dead ends / bugs:** (a) only 56 of 210 types had materials until the `BlockPrefabs` were found; (b) the first
"conflict" check compared material objects, but the scene keeps its own copies; comparing names leaves 4 real
conflicts; (c) type 0 briefly got a material from prefabs with `globalId` 0, caught by the real-data test;
(d) references to `library/unity default resources` needed mapping to `Resources/`.
**Not done / known issues:** no soft blending between materials, so borders follow the voxel grid and patchy
materials (red grass) look blocky; new roadmap item M6b. `_SIGMap` (gloss/emission) unused. Map mirroring still
unverified. Underwater look (absorption, fog colour) is M8.

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

**Dead end:** the first oracle diff failed only on Windows line endings (`\r\n` from Python), which had also
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
