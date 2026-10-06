# AGENTS.md

Rules for every AI agent (and human) working in this repository. Read this at
the start of every session. If a rule here conflicts with a request, stop and ask.

## Project

subnautica-rs is an unofficial, from-scratch Rust reimplementation of the
Subnautica engine that reads the player's own installed copy of the game at
runtime, runs natively on desktop, and supports self-hosted co-op
multiplayer. A browser/WASM build is explicitly out of scope for now.
Architecture and roadmap: `docs/DESIGN.md`.

## Hard rules — never break these

1. **No copyrighted game data in the repository. Ever.** No textures, models,
   meshes, audio, maps, octrees, shaders, prefabs, localisation text, scripts,
   decompiled code, or anything extracted, converted, or derived from the game
   (including OBJ/PNG/glTF exports, screenshots, and dumped tables). Game data is
   read from the install at runtime, or written to the gitignored `out/` folder.
2. **`.gitignore` is a whitelist.** It ignores everything and re-includes only
   our source and docs. Never convert it to a blacklist. Never whitelist binary
   file types. If a new file type is needed, add the narrowest pattern possible
   and say so in the change summary.
3. **The game install is read-only.** Never write, move, patch, or delete
   anything under the game directory. Never modify the game's executable or DLLs.
4. **No DRM or access-control circumvention.** We read files the player already
   has on disk. We do not touch Steam/DRM, anti-tamper, or licence checks.
5. **Multiplayer only through our own self-hosted server.** Never connect to,
   emulate, or scrape any official or third-party online service. The server
   must never send game assets to clients — every player supplies their own copy.
6. **Clean-room with respect to other projects.** We may *read* Nitrox,
   UnityPy, AssetStudio, Subnautica-TerrainPatcher, etc. to understand formats
   and architecture, then write our own code. Do not paste their code. If we ever
   reuse code, its licence must be compatible, and it must be recorded in
   `THIRD-PARTY-NOTICES.md` before the commit. (Nitrox is copyleft — assume its
   code cannot be copied.)
7. **No commits, pushes, or history rewrites unless the user asks.** Never use
   `git add -f` / `--force` to bypass the whitelist.
8. **No credentials** in the repo or in any file the agent writes.
9. **Stay inside this project folder** (plus read-only access to the game
   install and system temp). Don't edit user configs elsewhere.

Before finishing any task that touched files, run `git status --short` and
confirm that nothing outside our own code/docs is listed.

## Architecture rules

See `docs/DESIGN.md` § Architecture for the full picture. The short version:

- **Layering is enforced.** Crates in layers 0–2 (core, formats, algorithms,
  protocol) must not depend on Bevy, wgpu, winit, tokio, or `std::fs`/`std::net`.
  They take bytes (`&[u8]`) and return data, so they can be tested headlessly.
  File access goes through `sn-install` (layer 3) or the apps.
- **Desktop only.** Don't add WASM/browser targets, `cfg(target_arch = "wasm32")`
  branches, or web dependencies.
- **Parsers never panic on bad input.** Malformed data returns an error with
  the byte offset. No `unwrap()` / indexing on untrusted data; use checked reads.
- **Only `sn-render` and the apps may depend on Bevy.**
- **Create crates when a milestone needs them, not ahead of time.**
- **Don't add a dependency without a one-line reason** in the change summary.
  Prefer small, pure-Rust crates.

## Testing rules

- **`cargo test --workspace` must pass on a machine without Subnautica.** Unit
  tests use synthetic fixtures *generated in code* (e.g. our own octree encoder),
  never files copied from the game. No binary fixture files.
- **Real-data tests are opt-in:** mark them `#[ignore]`, read the game path from
  the `SUBNAUTICA_DIR` environment variable, and skip cleanly if it is unset.
  Run with `cargo test -- --ignored`.
- Real-data tests may assert numbers (counts, sizes, hashes) learned from the
  game, but must not embed game content (byte blobs, strings, geometry).
- **Log, don't look.** The agent can't see the screen. Verify with numbers:
  counts, histograms, bounds, timings, invariants. Visual checks are a bonus for
  the human, not the primary evidence.

## How to work

- **Plan before code.** Anything bigger than a small fix gets a plan in
  `docs/DESIGN.md` (or a milestone note) first.
- **One milestone at a time, one concern per change.** Each change should be
  revertable on its own.
- **Every milestone has a "Done when" checklist.** Don't call it done until each
  item has been run and passed.
- **After every change, give the exact command(s) to verify it** and what output
  to expect.
- **Ask before large refactors** or before changing the architecture.
- **If stuck after two genuine attempts, stop** and write `STATUS.md` (what was
  tried, what happened, current hypothesis) instead of trying random variations.
- **Document format findings as prose in `docs/formats/`** — that is the
  shareable output of reverse engineering. Mark each fact as *confirmed* (with
  how) or *hypothesis*.

## Honesty

- If something is not tested, write **"not tested"**.
- If unsure, say so. A confident wrong answer costs hours.
- Record dead ends in `MODLOG.md`, not just successes.

## Keep these files updated

- `MODLOG.md` — one entry per change: what, why, how it was verified.
- `docs/DESIGN.md` — when architecture or roadmap changes.
- `docs/formats/*.md` — whenever we learn something about a file format.
- `README.md` — the "what works / what doesn't" list (only tested claims).

## Environment

- OS: Windows 11 (primary dev machine). Linux/macOS: unverified.
- Rust: stable, currently 1.96 (MSVC toolchain).
- Game: Subnautica (Steam, Windows), StreamingAssets `__buildnumber.txt` = `10`,
  `__buildtime.txt` = `10/03/2025 17:26:27`, world data dir `SNUnmanagedData/Build18`.
- Game path on the dev machine: `D:\SteamLibrary\steamapps\common\Subnautica`
  (pass via `SUBNAUTICA_DIR` or `--game-dir`; never hard-code it in source).
- Generated output (meshes, images, dumps): `out/` — gitignored, never committed.
