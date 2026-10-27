# M9e3 checkpoint — stopped after two close-pose checks

2026-10-27, main checkout. M9e1/M9e2 remain complete. M9e3 is **not
done**; M9e4 and M9e5 were not started. No branch was merged or pushed.

## What is implemented

- Runtime pod fabricator placement, hand colliders, localized hand text,
  strict close-distance validation on the server, progress hand reticle.
- Pure range/lid/beam intersection rules with synthetic tests.
- Craft output preview from the selected, activated model subtree; owner
  automatically requests collection on completion in range.
- Logged plain-material preview and plain geometry beams/sparks. The
  fabrication shader, particle simulation and fabrication light are unported.
- Scoped reuse of the existing optimized-avatar restoration for Fabricator,
  including skin palette restoration. Inspector bindings improve from 0/120
  to 120/120. Virtual avatar nodes have no serialized components.
- Preview parenting to the exposed animated itemSpawnPoint.

## Verified before the avatar change

`cargo test --workspace`: 752 passed, 65 ignored. Clippy with warnings
denied and formatting check passed. The ignored install read test resolved
90/90 output classes (88 VFX models, two without a model; two inactive roots).

With SUBNAUTICA_DIR set to the installed game, release client:

```
target/release/sn-client.exe --view 400 --resolution 960 540 --test-supply ScrapMetal --use-item 3 --panel 6=take-all,7=close --craft 8=open,9=Titanium --shot 14 m9e3-solo
target/release/sn-client.exe --view 400 --resolution 960 540 --craft 3=open --hold-back 6 --shot 12 m9e3-close
```

Both exited successfully with zero WARN/ERROR. Solo: hand text, opening at
0.8086 m, one preview entity drawn at world event 10, two beams/six sparks
each, four Titanium collected at 2.8 s, preview removed at event 12, power
75 to 70. Walk-away: logical lid opens at 2.0634 m and closes at 3.0057 m
against stored distance 3 m. Screenshots/logs are ignored under out/.
These initial runs exposed that no fabricator animator had been bound.

## Blocking check and two attempts

The real-data test `fabricator_world` now also restores/binds the avatar,
opens the animator for 180 frames at 60 Hz, closes for 240 frames, and
compares its final pose with a sampled initial closed pose.

1. Raw component comparison: opening changes the pose by 0.7428153;
   closing residual 1.4142408, failing the 1e-4 bound.
2. Account for equivalent quaternion signs per rotation slot: opening
   still changes by 0.7428153; closing residual 0.031879187, still fails.

Following AGENTS.md's two-attempt rule, implementation stopped here. The
ignored test retains the failing assertion as a reproducible check; it
must not be described as passing. Reproduce in PowerShell:

```
$env:SUBNAUTICA_DIR='D:\SteamLibrary\steamapps\common\Subnautica'
cargo test -p sn-assets fabricator_world -- --ignored --nocapture
```

**Current hypothesis, not confirmed:** the initial closed sample and final
closed sample may use different clip times, or some curve/default channel
may retain a value through the cycle. Quaternion sign explained most,
but not all, of the difference. The remaining slot has not been identified.
Inspect per-slot differences and controller state/times before changing
playback or weakening the assertion. Original-game screen parity is
**not tested**. M9e3 cannot receive a done mark until the animation is
understood and its full Done-when list passes on the final code.

Other dead ends: inverse placement failed on non-uniform prefab transforms
(replaced with subtree detachment/rebasing); restored virtual bones first
caused component lookup in a nonexistent serialized file (fixed by treating
virtual nodes as component-free); a Python launch with a relative executable
path failed (absolute path works). One concurrent workspace compilation
ended with rustc STATUS_STACK_BUFFER_OVERRUN; final checks were retried with
CARGO_BUILD_JOBS=2. Final check results are recorded in MODLOG.md.

## Final checkpoint verification

The final release build succeeds. Default workspace tests: 752 passed,
65 ignored; Clippy with warnings denied and formatting check pass.
Final client solo and walk-away runs exit 0 with zero WARN/ERROR. One
fabricator animator is bound; the preview attaches to its animated spawn
point and is removed on collection. Solo opens at 0.8826 m; walk-away
closes the logical state and animator parameter at 3.0754 m (limit 3 m).
These checks do not resolve or replace the failing pose-cycle assertion.
Final logs/screenshots use out/m9e3-*-final.*.

M9e4 menu, recipe tooltip/click/unlock integration and M9e5 two-player
crafting are **not tested** and remain unimplemented. No dependencies or
whitelist patterns were added. Generated meshes, screenshots and logs stay
in gitignored out/.
