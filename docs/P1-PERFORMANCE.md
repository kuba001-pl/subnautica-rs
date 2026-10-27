# P1 client performance evidence (2026-10-27)

Perf P2 continuation is recorded at the end of this document. P1 measurements
below are historical evidence, not a P2 baseline.

P1b removes main-thread texture conversion stalls. P1c inactive UI pruning
was rejected after repeat measurements failed to establish a net frame-time win.
The retained change preserves the scene and game rules. A universal
FPS target is not claimed. Machine: i9-10900K, RTX 3080, Vulkan, Windows 11.
All logs, samples, binaries and captures stay in ignored `out/p1/` and
`out/baselines/`; none are repository fixtures. No dependencies or whitelist
changes. The game install is read-only.

## Ranking and changes

1. **Asset bursts on the main thread.** In the player script,
   `objects::stream_objects` worst was 39.214-49.650 ms. Late texture
   construction did 129 conversions, about 35-43 ms total. P1b moves the
   unchanged CPU image conversion to the existing asset worker. After:
   0.727-0.779 ms worst in that system across three runs; the 129 conversions
   remain on the worker. GPU upload stays with Bevy. Player whole-run maximum
   falls from 97.468-119.382 to 64.023-69.920 ms; these maxima also include
   screenshot and exit work and are not pure gameplay maxima.
2. **Rejected P1c: UI builds process inactive hierarchies.** P1c retains layout but skips
   graphics/text mesh, material and glyph work that `draw_list` discards for
   inactive nodes. Immediate pre-fix worker means: 3.600-3.638 ms fixed,
   3.943-3.988 player; after: 1.672-1.859 fixed, 2.228-2.410 player.
   This is about half the CPU build work, despite noise in the later runs.
   Reverted: worker savings alone do not establish a net gameplay win.
   Skipping inactive nodes also defers texture/glyph preparation until activation;
   its contribution to the observed tails is a hypothesis, not confirmed.
3. **Other measured recurring work is smaller.** Before fixes, player
   animation mean 0.295-0.299 ms, simulation 0.272-0.278, LOD 0.136-0.142,
   UI GPU-buffer preparation 0.200-0.206, UI draw recording 0.168-0.172,
   terrain streaming 0.008-0.009. Fixed-camera recent GPU pass averages in
   the first run: opaque 0.870 ms, shafts 0.835, bin unpacking 0.459,
   transparent 0.446. Recent CPU pass averages in a supplemental baseline:
   transparent 0.445 ms, opaque 0.286. These are nested/partial scopes, not
   an additive accounting of an entire frame.

The initial benchmark panicked before any samples: the PDA pose system
required a player-only resource in free-camera mode. P1a makes it optional
and adds a synthetic regression test, then adds the opt-in profiler.
The failed attempt is recorded as `p1-before-fixed-1.log`, never as timing
evidence. No subsequent panic/warning/shader error in the measured runs.

## Matched runs

Each row lists runs 1 / 2 / 3 in milliseconds. All fixed-camera runs measured
600 frames after 120 settled warm-up frames, with zero unsettled samples.
All use view 400, 1600 x 900, fixed seeds, noon/frozen game clock, intact
Aurora, high water, grading off. Animation/waves/foam remain live. Fixed
camera is uncapped (`AutoNoVsync`); the player retains `AutoVsync`.
Player samples start at play >= 1 s and include late loads and capture/exit.

| Build/case | p50 | p95 | p99 | max |
|---|---|---|---|---|
| baseline fixed | 9.419 / 9.533 / 9.544 | 10.631 / 10.674 / 10.746 | 11.246 / 11.380 / 11.353 | 12.368 / 12.403 / 12.518 |
| baseline player | 10.401 / 10.371 / 10.449 | 11.708 / 11.669 / 11.957 | 15.460 / 13.095 / 12.983 | 119.382 / 97.468 / 98.266 |
| textures fixed | 9.619 / 9.424 / 9.302 | 11.040 / 10.632 / 10.348 | 11.718 / 11.355 / 10.674 | 13.715 / 13.818 / 11.385 |
| textures player | 10.337 / 10.430 / 10.372 | 11.949 / 11.674 / 12.010 | 13.059 / 12.716 / 16.910 | 66.503 / 69.920 / 64.023 |
| ui fixed | 9.257 / 9.579 / 10.166 | 10.304 / 11.377 / 12.381 | 10.587 / 12.196 / 22.617 | 11.910 / 14.147 / 29.121 |
| ui player | 10.642 / 10.860 / 11.518 | 15.852 / 14.629 / 18.793 | 37.423 / 23.167 / 25.073 | 76.632 / 66.851 / 67.019 |

Baseline is P1a, textures is P1b, ui is the rejected P1c candidate. UI before/after compares
textures versus ui. The later UI series has worse whole-frame tails while
its worker cost is consistently lower. External CPU/GPU activity and OS
scheduling are possible causes, not proven; preserve the noisy evidence.
Do not infer an FPS win from the worker timer alone.

The fixed-camera static snapshots are exactly equal across all nine runs
(and the supplemental diagnostic run): 103 batches, 4,393,498 shown
triangles, 589 terrain meshes, 25,311 grass tufts / 1,970,238 grass triangles;
17,194 object entities, 237 prefabs, 2,599 object meshes, 598 materials,
1,079 object textures; 54 point, 15 spot, 4 directional lights; 4,591 LOD
groups. Camera, sky and fog fields match too. Animated geometry and final
images are not bitwise deterministic. Each player run takes all six locker
items and refuses the right-click drop in the pod.

## Default view distance and normal run profile

The same fixed-camera flags at `--view 1200` (three runs each):

| Build/case | p50 | p95 | p99 | max |
|---|---|---|---|---|
| default-beforeui | 17.466 / 17.374 / 17.392 | 18.720 / 18.937 / 19.716 | 19.312 / 19.465 / 22.667 | 20.564 / 20.236 / 28.777 |
| default-final | 17.461 / 17.459 / 17.842 | 19.270 / 19.952 / 21.052 | 20.700 / 21.924 / 22.562 | 22.152 / 25.314 / 27.107 |
| default-dev | 19.907 / 21.916 / 21.622 | 41.242 / 28.408 / 25.595 | 71.081 / 54.695 / 33.707 | 168.225 / 156.111 / 157.905 |

`default-beforeui` is the saved P1b release executable; `default-final` is
P1c release; `default-dev` is P1c built with `cargo build -p sn-client`.
The documented ordinary solo command uses dev: workspace opt-level 1,
dependencies opt-level 3. These are already partially optimized builds.
The dev medians were higher, but this is not a clean causal profile A/B:
another `sn-client.exe` was present before dev run 3, and starts of earlier
runs do not rule out activity during them. All these runs record counts of
other clients, cargo, rustc and Subnautica processes before launch. No cargo,
rustc or original-game process was detected at those starts. Preserve the
large tails, do not attribute every stall to compilation profile. Release is
the performance/play recommendation; existing dev settings are retained.

At this view distance the UI worker mean drops from 3.689-3.784 ms to
1.718-1.744 ms, while whole-frame release medians remain 17.459-17.842 ms
(about 56-57 FPS). The 16.7 ms / 60 FPS threshold is still not met. The
remaining whole-frame cost is not fully attributed by these partial timers;
further engine extraction/visibility/submit/present and full-GPU timing is
needed before claiming the steady-FPS issue solved. Reducing view distance
changes drawn content, so it is not used as a parity-preserving fix.

Static snapshots match across all nine default-view reports, excluding the
non-drawn LRU cache occupancy: 1,393 shown batches, 7,759,704 shown triangles,
2,083 shown terrain meshes, 34,469 grass tufts, 39,353 object entities,
12,493 object meshes, 1,968 materials, 2,265 object textures; 508 point,
51 spot and 4 directional lights. All have zero unsettled frames and
warnings, and exit 0. No claim about the original game's performance.

## P1c decision: rejected (repeat investigation)

Three rounds alternated P1b eager fixed/player, then candidate pruned
fixed/player, using the exact protocol above and archived executables
`out/p1/textures-client.exe` / `target/release/sn-client.exe`. Logs:
`out/p1/p1-matched-{eager,pruned}-ROUND-{fixed,player}-1.log`.
No concurrent client or Rust compiler was observed by the runner.

| Build/case | p50 | p95 | p99 | max |
|---|---|---|---|---|
| repeat eager fixed | 11.244 / 9.548 / 10.917 | 37.250 / 10.934 / 12.294 | 74.228 / 11.864 / 13.032 | 112.769 / 14.443 / 13.706 |
| repeat eager player | 11.897 / 10.347 / 10.915 | 13.785 / 11.688 / 12.332 | 15.809 / 12.925 / 13.604 | 70.076 / 71.802 / 65.299 |
| repeat pruned fixed | 12.299 / 9.266 / 9.864 | 35.631 / 10.263 / 11.034 | 73.900 / 10.739 / 11.462 | 95.320 / 11.560 / 12.023 |
| repeat pruned player | 11.665 / 13.294 / 10.987 | 13.940 / 27.911 / 18.038 | 18.638 / 52.213 / 22.541 | 74.995 / 118.593 / 66.735 |

Both first fixed runs have p99 near 74 ms, so a large part of the instability
is shared across builds. The candidate still fails to show a reliable gameplay
tail win. CPU build savings and 32-state / 2,004-draw installed-data parity are
real, but insufficient for the keep criterion. The source and oracle test were
reverted; P1b's eager UI path remains. This is a dead end, not a retained fix.
These legacy player rows include capture/exit and must not be called gameplay
maxima. Later attribution must explicitly stop sampling before capture.

## P1d: render-stage spike attribution and object GPU upload budget

`SN_CLIENT_PERF=1` now stops sampling at `play_shot` before screenshot
readback/exit, writes `out/p1/frames.csv` and `out/p1/spans.csv`, and times
`Render` schedule sub-stages (`render::assets`, `render::images`,
`render::mesh_alloc`, `render::meshes`, `render::specialize`, `render::queue`,
`render::prepare`, `render::render`) alongside main-thread asset work counters.

### Spike attribution (excluding capture/exit)

1. **Object asset burst in `objects::stream_objects` (player + swim):**
   `library.scenes` and `library.batch` buffered all updates until the scene or
   batch completed, and `stream_objects` inserted every queued `Image` and
   `Mesh` in a single frame. In `player` at `play = 1.36–1.62 s`, `152.9 MB` of
   textures and `193,562` mesh vertices were extracted in one frame, stalling
   `render::assets` for `47.6–56.7 ms` (`render::images` `46.9–50.5 ms`,
   `render::mesh_alloc` `30.9–38.9 ms`) and producing the `64.7–70.8 ms` max
   frame spike. In the 35 s `swim` route (`--start -5 -10 -5 --look 200 -10 -5
   --hold-forward 1 --turn 8 --shot 35`) at `play = 3.94–3.98 s`, `45.0 MB` of
   textures and `401,807` mesh vertices stalled `render::assets` for
   `47.1–49.0 ms` (`75.0–78.7 ms` frame spike).
2. **LOD-0 terrain + grass batch upload in `terrain::stream` (swim):**
   Four LOD-0 batch completions at `play = 7.64–9.00 s` and `32.40–33.48 s`
   each insert `668k–848k` terrain vertices plus `521k–736k` grass vertices
   (`1.19M–1.58M` vertices in one frame), stalling `render::mesh_alloc` for
   `103.1–141.7 ms` (`125.5–174.8 ms` frame spikes; addressed in P1e).

### P1d fix and 3-run matched results (`--view 400 --resolution 1600 900`)

`objects.rs` now sends updates from the worker as each asset is decoded (`Out`)
and drains `ObjectStreamer::pending` on the main thread under per-frame upload
caps (`TEXTURE_UPLOAD_BYTES_PER_FRAME = 8 MiB`,
`MESH_UPLOAD_VERTICES_PER_FRAME = 32,000`, `SPAWN_BUDGET = 1,500`), while
`ObjectStreamer::settled` requires `pending.is_empty()`.

| Case / metric | Before P1d (3 runs, ms) | After P1d (3 runs, ms) |
|---|---|---|
| fixed (`600` frames) p50 | 9.776 / 9.789 / 9.736 | 10.230 / 9.847 / 9.773 |
| fixed (`600` frames) p95 | 10.859 / 10.541 / 10.712 | 12.436 / 11.884 / 10.998 |
| fixed (`600` frames) p99 | 11.469 / 11.335 / 11.166 | 14.109 / 13.694 / 11.826 |
| fixed (`600` frames) max | 11.910 / 11.984 / 12.223 | 17.956 / 15.741 / 12.434 |
| player (`--shot 12`) p50 | 10.694 / 11.366 / 10.704 | 10.812 / 11.726 / 10.566 |
| player (`--shot 12`) p95 | 12.190 / 26.040 / 12.123 | 12.423 / 25.528 / 11.997 |
| player (`--shot 12`) p99 | 13.816 / 29.635 / 13.425 | 15.852 / 29.413 / 14.340 |
| player (`--shot 12`) max | 70.822 / 66.462 / 69.706 | 27.872 / 36.735 / 28.438 |
| player `render::assets` max | 56.673 / 48.557 / 55.165 | 5.719 / 6.519 / 5.863 |
| swim (`--shot 35`) p50 | 9.269 / 9.707 / 9.321 | 8.967 / 8.887 / 10.194 |
| swim (`--shot 35`) p95 | 11.817 / 13.260 / 14.181 | 11.179 / 11.097 / 17.421 |
| swim (`--shot 35`) p99 | 20.981 / 16.228 / 18.582 | 12.642 / 12.590 / 20.997 |
| swim (`--shot 35`) max | 166.824 / 166.956 / 174.818 | 168.571 / 171.400 / 169.033 |
| swim `render::images` max | 17.646 / 18.530 / 17.458 | 4.244 / 3.608 / 5.216 |
| swim `play~3.95 s` object burst | 76.129 / 74.958 / 77.408 | 24.290 / 25.000 / 31.679 |

All six fixed-camera static snapshots match byte-for-byte; all six player runs
perform the six locker `switch` moves and refuse the pod drop with zero warnings.

## P1e: staged LOD-0 terrain + grass mesh upload and intra-slot object spawn budget

In the 35 s `swim` route across streaming boundaries, each LOD-0 terrain batch
completion previously uploaded all `done.parts` (`668k–848k` vertices, single
base layer up to `261,530` vertices) and `done.grass` (`521k–736k` vertices,
single grass layer up to `583,375` vertices) in one frame, stalling
`render::mesh_alloc` for `103.1–143.7 ms`, while `objects::stream_objects`
spawned the batch's entire fine-detail (`level == 0`) slot (`2,118–4,914`
entities, `8.4–20.9 ms`) in the same frame because `SPAWN_BUDGET` was checked
only between slots.

`terrain.rs` now splits oversized `TerrainPart` and `GrassPart` meshes on the
worker thread into chunks of at most `PART_TRIANGLE_BUDGET = 12,000` triangles
(`<= 36,000` vertices, preserving shared batch bounding boxes and counting one
logical mesh per layer in `StreamStats::shown_meshes`) and stages in-place LOD
transitions across frames in `TerrainStreamer::staging` under
`STAGED_VERTEX_BUDGET = 48,000` vertices per frame, swapping the batch's
entities on the frame after its GPU buffers finish allocating so the old LOD
remains on screen without holes. `objects.rs` tracks `spawn_cursor` per
`(batch, level)` slot and lowers `SPAWN_BUDGET` to `600` entities per frame so
dense fine-detail slots are spread across frames.

| Case / metric | Before P1e / After P1d (3 runs, ms) | After P1e (3 runs, ms) |
|---|---|---|
| fixed (`600` frames) p50 | 10.230 / 9.847 / 9.773 | 9.852 / 9.895 / 9.859 |
| fixed (`600` frames) p95 | 12.436 / 11.884 / 10.998 | 10.763 / 11.163 / 10.848 |
| fixed (`600` frames) p99 | 14.109 / 13.694 / 11.826 | 11.434 / 11.796 / 11.505 |
| fixed (`600` frames) max | 17.956 / 15.741 / 12.434 | 12.659 / 16.137 / 11.759 |
| player (`--shot 12`) p50 | 10.812 / 11.726 / 10.566 | 10.714 / 10.791 / 10.660 |
| player (`--shot 12`) p95 | 12.423 / 25.528 / 11.997 | 12.389 / 12.437 / 12.201 |
| player (`--shot 12`) p99 | 15.852 / 29.413 / 14.340 | 14.301 / 13.974 / 14.628 |
| player (`--shot 12`) max | 27.872 / 36.735 / 28.438 | 22.645 / 26.272 / 28.515 |
| player `objects::stream_objects` max | 2.460 / 2.264 / 2.296 | 0.503 / 0.488 / 0.518 |
| swim (`--shot 35`) p50 | 8.967 / 8.887 / 10.194 | 8.989 / 8.984 / 8.967 |
| swim (`--shot 35`) p95 | 11.179 / 11.097 / 17.421 | 11.783 / 11.517 / 11.562 |
| swim (`--shot 35`) p99 | 12.642 / 12.590 / 20.997 | 16.358 / 16.317 / 16.494 |
| swim (`--shot 35`) max | 168.571 / 171.400 / 169.033 | 25.256 / 26.508 / 23.447 |
| swim `render::assets` max | 127.600 / 126.900 / 144.364 | 11.021 / 11.635 / 11.083 |
| swim `render::mesh_alloc` max | 123.300 / 122.900 / 143.738 | 8.014 / 7.957 / 7.901 |
| swim `objects::stream_objects` max | 14.316 / 13.904 / 20.892 | 1.703 / 2.015 / 1.766 |

All nine fixed-camera static snapshots (`p1d-before-1..3`, `p1d-after-1..3`,
`p1e-after-1..3`) match byte-for-byte (`103` batches, `4,393,498` terrain
triangles, `589` terrain meshes, `25,311` grass tufts, `1,970,238` grass
triangles, `17,194` object entities, `0` warnings). All three P1e `swim` runs
reach the identical end-of-route scene (`102` batches `[4, 4, 20, 74]`,
`2,950,338` terrain triangles, `16,852` grass tufts, `1,341,147` grass
triangles, `13,171` object entities, `201` slot objects, `0` warnings) with zero
frames `>= 33.333 ms` across `3,630–3,635` sampled frames.

## Reproduction

Run from the worktree in PowerShell; choose new baseline/capture/log names
for each repeat and label the actual source revision. Use the same executable
and flags before and after each independent fix.

```powershell
$env:SUBNAUTICA_DIR='D:\SteamLibrary\steamapps\common\Subnautica'
$env:SN_CLIENT_PERF='1'
New-Item -ItemType Directory -Force out/p1 | Out-Null
cargo build -p sn-client --release
$common=@('--view','400','--resolution','1600','900','--lifepod-seed','1','--slot-seed','1','--time','12','--time-scale','0','--aurora','intact','--water-quality','high','--color-grading','off','--gpu-timings')
1..3 | ForEach-Object {
    .\target\release\sn-client.exe @common --benchmark 600 --baseline "p1-repro-fixed-$_" --baseline-revision working-tree --warmup 120 --start 0 -10 0 --look 60 -40 60 --fov 60 *> "out/p1/repro-fixed-$_.log"
}
1..3 | ForEach-Object {
    .\target\release\sn-client.exe @common --use-item 3 --panel 6=take-all,7=drop,9.5=close --shot 12 "p1-repro-player-$_" *> "out/p1/repro-player-$_.log"
}
```

Expected: exit 0, zero warnings/panics/shader validation errors; report and
600 raw frame samples in each fixed baseline directory; `perf:` count,
mean/p50/p95/p99/max and histogram bins on exit. The `other` series covers
loading/warm-up; `sampled` covers benchmark frames or player play >= 1 s.
System spans include OS descheduling; threads/spans overlap. Each series
retains at most 120,000 samples. Recent Bevy CPU/GPU diagnostics are a
separate history window. The profiler is off without `SN_CLIENT_PERF=1`.

For the default-view comparison replace `400` in `$common` with `1200`
and choose fresh names. Build dev with `cargo build -p sn-client` and replace
the executable with `.\target\debug\sn-client.exe`; keep all flags unchanged.
Before/after UI revisions are P1b and P1c; initial texture-stall before/after
revisions are P1a and P1b. Do not mix view distances in an A/B comparison.

## Checks and limits

`cargo test --workspace` with the game environment variable unset: 724 passed,
0 failed, 63 ignored. `cargo clippy --workspace --all-targets -- -D warnings`
and `cargo fmt --all -- --check` pass. With the install configured:

```powershell
cargo test -p sn-assets active_only_draw_matches_eager_visible_output -- --ignored --nocapture
.\target\release\sn-inspect.exe ui --inventory
.\target\release\sn-inspect.exe ui --pda
.\target\release\sn-inspect.exe ui --draw
.\target\release\sn-inspect.exe ui --icons
.\target\release\sn-inspect.exe ui --popup
```

Expected/results: parity 32 states / 2,004 visible draws exact; inventory
53/53; PDA 173/173; draw hashes 12/12 with 0 failures; icons 10/10 / 80 draws
exact; pop-up 13/13 / 240 draws exact. The opt-in UI parity test compares the
eager and pruned visible draw data with one shared/prewarmed glyph atlas,
normalizing only unused material-table numbering. Cold glyph allocation can
place the same glyphs at different atlas coordinates. Layout and runtime
activation rules are unchanged. Existing inventory/PDA/icon/pop-up checks
cover their runtime behavior separately.

No original-game on-screen comparison, real mouse play, multiplayer
performance, Linux/macOS, deep-biome routes, long-distance streaming,
shader-compilation time isolation or full CPU/GPU scheduling accounting was
tested by this investigation. Initial loading, active UI rebuilds/uploads
and remaining whole-frame/capture stalls are not eliminated. Benchmark
warm-up settles terrain/objects, not every possible shader pipeline.

Evidence provenance: baseline/textures/ui and prior UI parity are inherited
from the previous agent; matched-eager/pruned runs and c-test-final /
c-clippy-final were run by the continuation. Repeat static snapshots match
exactly across all six fixed runs; all twelve runs have zero warnings/errors
and every player run performs six item moves.

## Perf P2 (2026-10-27): default view and PDA activation

Plan and numerical acceptance targets: DESIGN § Perf P2. P2a adds attribution
probes without changing drawing or simulation. `main::schedules` covers First
through Last (including nested systems); it excludes pipelined extraction,
rendering and presentation outside those schedules. Player reports now retain
recent GPU scopes as well as CPU scopes every 120 frames. `game_ui` GPU timing
covers its render pass. UI counters record actual submitted indexed draws,
texture binding groups and newly queued pipeline variants. The baseline mesh
census counts main-world `ViewVisibility`, including any view; these instance
and unique-mesh counts are **not** GPU draw-call or instancing statistics.

Reproduce P1's commands above with `--view 1200` and fresh `p2-*` names;
preserve `out/p1/frames.csv` and `spans.csv` under separate `out/p2/` names after
each player run (the profiler overwrites them on exit). Keep three repetitions
per case/build, record other clients/compiler processes, and compare the same
camera/seeds/resolution/effects. PDA activation attribution uses the scripted
3 s use-item event, independently from the 6 s take-all and 9.5 s close events.
Capture/exit remain excluded. Targets and runtime verification: **not tested**
until the repeated results below are recorded.

### P2b — direct instanced submission for CPU-culled views

The default camera has CPU frustum/range culling and no GPU occlusion culling.
Bevy 0.19's indirect path nevertheless rebuilds bin buffers and preprocessing
bind groups for 503 opaque + 137 alpha-mask + 106 shadow batch sets each frame.
Use `NoIndirectDrawing` on the camera: Bevy retains mesh/material instancing,
the existing CPU visibility rules, all four shadow views, and every effect.
`SN_CLIENT_INDIRECT_DRAWING=1` restores the old path for repeat attribution.
No dependencies, whitelist changes, asset changes or game-rule changes.

Initial paired rounds (`out/p2/r{0,1,2}-{baseline,direct}-fixed-1.*`) used one
archived release executable, switching only the camera flag. Order was B/A,
A/B, B/A. Each measured 600 frames after 120 consecutive settled frames.
All six had zero unsettled samples and identical drawn scene counts. Concurrent
clients/compiler processes were present; round 1 suffered extreme contention,
including a 149.947 ms recent opaque GPU scope. Keep that result visible rather
than treating its before/after ratio as the optimization's intrinsic speedup.

| Metric, ms; rounds 0 / 1 / 2 | Indirect before | Direct after |
|---|---|---|
| Frame p50 | 36.227 / 176.883 / 36.112 | 23.570 / 36.502 / 24.823 |
| Frame p95 | 63.042 / 305.159 / 121.890 | 42.165 / 91.648 / 43.444 |
| Frame p99 | 114.365 / 402.090 / 189.644 | 63.967 / 133.021 / 68.616 |
| Render batching mean | 8.889 / 30.225 / 14.713 | 2.123 / 8.772 / 2.360 |
| Bind-group preparation mean | 4.983 / 18.152 / 7.810 | 0.611 / 3.658 / 0.640 |
| Render graph mean | 23.591 / 36.526 / 23.217 | 21.113 / 26.101 / 21.430 |

Normal-round stage medians are less sensitive to descheduling: batching
7.678 / 7.913 → 1.729 / 1.850 ms; bind groups 4.283 / 4.332 → 0.422 / 0.453 ms.
Opaque CPU encoding rises (more direct API calls); the savings come from
removing indirect preprocessing, buffer/bind-group churn and bin unpacking,
not reducing mesh draws. The old path has 746 multi-draw batch sets; the direct
path has 1,814 opaque + 298 mask draws, and shadow draws [52, 53, 232, 290].
The transparent phase retains 1,071 sorted items (items are not draw calls).
A supplemental post-preparation census records effective indirect draw slots
as well as direct bins so these different submission APIs can be compared.

Static parity in all six reports: 1,393 terrain batches [8, 35, 205, 1,145],
7,759,704 terrain triangles, 2,083 logical terrain meshes; 34,469 grass tufts,
2,891,017 grass triangles; 39,353 object entities, 12,493 object meshes,
1,968 materials, 2,265 textures; 37,183 mesh instances, 6,171 view-visible
instances and 2,765 visible unique meshes; face census 2,819 off / 31,687 back,
524 reflected instances / 325 reversed variants / 0 unsupported. LRU cache
occupancy is not drawn content and varies. Four shadow views are preserved.

Direct-path recent GPU scopes across these rounds, ms: opaque 2.981–3.882,
transparent 1.209–1.480, shafts 1.293–2.134, UI 0.194–0.449, water surface
0.218–0.318, FFT 0.162–0.279, clustering 0.084–0.133. Bin unpacking disappears.
The instrumented GPU-scope sum is 6.472–9.094 ms versus 21.113–26.101 ms CPU
render graph wall time: the normal default-view cases are CPU/driver limited.
These scopes are recent windows, not total GPU-frame timings. Separate GPU
shadow timing and a full heap-allocation trace are **not tested**; the inherited
Bevy trace attributes 0.290 ms/frame CPU shadow recording under its own load.
GPU buffer/bind-group churn is attributed through batch-set counts and stage
costs, not presented as a count of all CPU heap allocations.

The absolute 16.667 ms target is not met by these contended runs. Workspace tests (748 passed, 64 ignored), clippy with warnings denied,
and fmt pass (`out/p2/p2b-validation.log`). Supplemental counted runs
`census-{baseline,direct}-fixed-1` confirm exactly 1,814 opaque, 298 mask and
627 shadow effective mesh draws in both paths; 1,071 transparent sorted items.
Direct instancing contains 3,926 opaque, 713 mask and 1,392 shadow instances.

Reproduce from PowerShell using the common flags in § Reproduction, with
`--view 1200`, `SN_CLIENT_PERF=1`, and a release build. For each of three rounds,
run the fixed command once with `$env:SN_CLIENT_INDIRECT_DRAWING='1'` (before)
and once after `Remove-Item Env:SN_CLIENT_INDIRECT_DRAWING -ErrorAction
SilentlyContinue` (after), alternating order. Use new baseline/log names and
archive profiler CSVs after each run. Expected: zero unsettled samples; scene
and effective draw counts above, with bin unpacking present only before.
Actual timing depends strongly on concurrent GPU/compiler work.
