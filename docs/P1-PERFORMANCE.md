# P1 client performance evidence (2026-10-27)

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

`cargo test --workspace` with the game environment variable unset: 722 passed,
0 failed, 64 ignored. `cargo clippy --workspace --all-targets -- -D warnings`
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
