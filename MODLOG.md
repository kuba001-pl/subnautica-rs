# MODLOG

One entry per change: what, why, how it was verified. Record dead ends too.

## 2026-10-10 — M9b: the player (walk, swim, the lifepod's hatches)

**What:** fourth step of Phase E (`docs/DESIGN.md` § 4.3, "M9b plan" and
"as built").
- `sn-unity`: parsers for `GroundMotor`'s own fields, `UseableDiveHatch`,
  `CinematicModeTrigger` (with Unity's serialized event lists),
  `PlayerCinematicController` and `EscapePodFirstUse`; new `physics`
  module: `PhysicsManager` (with the layer collision matrix),
  `TimeManager`, `TagManager`. Each is checked against the exact byte
  length on real data. Synthetic tests check them on every truncation.
- `sn-assets`: `player_data` gains the walking motor, the rigid body, the
  player's layer and the ocean level; `physics_settings`; scene functions
  for the pod's triggers, dive hatches and the first-use swap; new
  `collision_world`: the batch loader moved out of `sn-inspect swim`
  (terrain, objects, placeholder spawns, layer rules), the lifepod's
  colliders and triggers, and the player's parameters. Mesh colliders
  with a mirroring scale swap their winding back (PhysX `flipsNormal`).
- `sn-sim`: `player` (state, parameters, input; one fixed step: motor
  choice at the game's water levels, swimming, walking through our own
  character controller with step offset, slope limit and ground check,
  jumping, the hand target, the hatch triggers). `collide`: triangles are
  now one-sided, bodies carry groups, plus `cast` and `crossings`.
- `sn-inspect`: `walk` (the scripted run) and `prefab --winding`;
  `player` prints the new data; `swim` uses the shared loader and counts
  surfaces passed through.
- `sn-client`: the player is the default (`--free-cam` keeps the fly
  camera). It has a first-person camera at eye height and WASD / Space /
  C / mouse controls. E or a left click uses a hatch trigger. Collision
  streams on a worker thread.
- Real-data test `player_movement_data`.

**Findings** (`docs/formats/gameplay.md` § Player movement):
- Layer 19 ("Player") collides with every layer but 9 ("OnlyVehicle"),
  and every collider loaded so far is on layer 0.
- The fixed time step is 0.02 s.
- PhysX 4.1 (Unity 2019.4.36f1) treats mesh colliders as one-sided in
  contacts, sweeps and rays. I read its BSD-3 source to check; nothing
  was copied.
- The pod is entered and left through 8 cinematic triggers. Its dive
  hatch is inactive.

**Dead ends:**
- The first controller stopped at a 0.3 m step's face. It never lifted
  the capsule, although a 0.4 m step offset should clear it. With that
  fixed, it then climbed a 0.6 m step. Both were traced in the
  controller's three passes (up, across, down) and fixed; the unit test
  covers 0.3 m (climbs) and 0.6 m (doesn't).
- The real hatch is not `UseableDiveHatch`, as the plan assumed. That
  node is inactive in the pod; the cinematic triggers do the job.
- `sn-client --benchmark` turns the player off (measuring runs keep the
  fixed camera), so the client check is a normal launch stopped after
  45 s.
- Winding rays from above deep terrain batches start inside rock and see
  back faces (up to 3,150 of 3,300 in one batch). The check uses shallow
  batches only, plus the `sn-mesh` orientation test.

**Verified (2026-10-10):**
- `cargo test --workspace` passes without the game, including `sn-sim`'s
  24 tests. They cover:
  - swim speed capped and drag stopping;
  - surface damping and the motor switch at the game's water levels;
  - walking on a floor, a 0.3 m step but not 0.6 m, sliding on 70°,
    jump height;
  - hand reach, and the hatch;
  - a one-sided wall that blocks from the front only.

  `sn-unity` adds tests for the new parsers, and `sn-assets` for the
  mirrored winding.
- `SUBNAUTICA_DIR=… cargo test -p sn-assets --test real_data -- --ignored
  player_movement_data player_and_pda_data lifepod_colliders` passes.
- `cargo run --release -p sn-inspect -- walk`: RUN OK. Leaves the pod at
  1.7 s through `bot_out_trigger_first` (first-use swap logged), swims
  71 m away in 10 s and boards through `bot_in_trigger` at 21.8 s. 1,138
  steps, 0 penetrations, 0 surfaces passed through, smallest gap 5 mm,
  5.9 µs mean per step. Speeds: walking max 3.50 (read 3.5); swimming
  7.22 (7.6 read, 7.22 after one step of drag).
- `sn-inspect swim --seed 1…5`: all arrive in Kelp Forest, 0
  penetrations, 0 surfaces passed through, smallest gap 5.1–5.2 mm,
  13–19 µs mean per step.
- `sn-inspect prefab --winding`: terrain 17,066 front / 1 back; mesh
  colliders 6,012 front / 152 back.
- `sn-client` (normal launch, 45 s, no input): the player is ready 4.3 s
  after start, collision for 4 batches in 2.0 s, and it stands on the pod
  floor at (−126.79, 1.83, −49.75), grounded, as in the headless walk.
- **Not tested:** playing with keyboard and mouse in the client (the
  agent can't give input); the look limits and mouse sensitivity are our
  own.

## 2026-10-10 — M9a: collision (capsule sweep, terrain and object colliders, scripted swim)

**What:** third step of Phase E (`docs/DESIGN.md` § 4.3, "M9a plan").
- New crate `sn-sim` (layer 2, pure, no dependencies): `collide`. A
  capsule is swept against triangles, spheres, capsules and oriented boxes
  by conservative advancement on exact distances, so nothing can tunnel
  through. `move_and_slide` makes up to 4 slides and keeps a 1 cm skin.
  `clearance` and `push_out` find and fix overlaps. Bodies are added and
  removed by id, with a uniform 4 m grid in each body.
- `sn-terrain::collision_triangles`: the level 0 batch mesh without
  skirts, as Unity-space triangles.
- `sn-assets::collision`: `prefab_colliders` keeps enabled, non-trigger
  colliders on active nodes and caches mesh-collider triangles.
  `PrefabCollider::world` applies Unity's scaling rules (hypothesis).
- `sn_world::Transform::transform_point` / `rotate_vector`.
- `sn-inspect swim [--seed N] [--seconds S]`: the scripted swim, headless.
  Terrain and objects (cells, slot spawns, batch objects) are loaded for
  the batches within 56 m of the player, the game's collision range.
- Real-data test `lifepod_colliders`.

**Findings** (`docs/formats/gameplay.md` § Collision): the game collides
with terrain only at clipmap level 0 (7×7×7 chunks of 16 voxels). There
the collision mesh is thinned by a native plugin we don't port. The
underwater player capsule is radius 0.3, height 0.75, centre 0.125 m
below the camera. The lifepod has 40 box colliders.

**Dead ends:**
- The first swim stayed at −20 m and touched nothing (0 contacts), so it
  tested nothing. The swim now aims below the seabed and slides along it.
- The first target was the nearest Kelp Forest map cell. It lies on the
  forest's edge (4 m cells), and 3 m short of it is Safe Shallows. The
  target now needs Kelp Forest 20 m around it, and the end point must be
  in Kelp Forest.
- Seed 1 got wedged for 590 s in a 0.5 m slot between a terrain wall and
  an overhanging object. That was correct collision: the slot is narrower
  than the capsule. The script now detours sideways after 1 s blocked.
- Two unit tests first expected the gap to stay ≤ `SKIN`. The 0.1 %
  overbounce lifts it a few mm more (by design), so the bound is now
  2 × `SKIN`.

**Verified (2026-10-10):**
- `cargo test --workspace` passes without the game. That includes
  `sn-sim`'s 12 tests: floor and wall slides, an inside corner, a thin
  wall at 100 m/s from both sides, each primitive's stopping distance, a
  rotated box, a bumpy height field, and 3,000 random moves in a closed
  room with obstacles with no penetration. Also the scaling rules,
  `world_triangles` and `transform_point`.
- `SUBNAUTICA_DIR=… cargo test -p sn-assets --test real_data -- --ignored
  lifepod_colliders` passes.
- `cargo run -p sn-inspect -- swim` (seed 1): ARRIVED in kelpForest after
  51.8 s simulated, 3,693 contacts (3,424 terrain, 269 objects), 0
  penetrations, smallest gap 5.3 mm, 30.5 µs mean / 118.9 µs p99 per
  step, batch loads about 0.6 s each. Seeds 2–5: all arrive, 0
  penetrations, smallest gap 5.1–5.3 mm.
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets
  -- -D warnings` are clean.
- **Not tested:** anything in the client (M9b uses this); the game's own
  collision for comparison; the physics layer matrix (not read); whether
  the game's triangles are one-sided.

**Follow-up (same day, user's request):** every M9a gap now has a home in
`docs/DESIGN.md`. M9b's row gains the physics layer matrix, the lifepod
module and placeholder colliders, and the one- or two-sided check (each
with a "done when" check). The terrain thinning plugin, the scaling check
and PhysX rigid-body response join the "Deferred, not dropped" list.

## 2026-10-10 — P1: our own .NET reader; tech type names, craft menus, TechData defaults

**What:** second step of Phase E (`docs/DESIGN.md` § 4.3).
- New crate `sn-dotnet` (layer 1, pure, no dependencies): PE container,
  CLI metadata (`#~` tables with the full ECMA-335 schema, `#Strings`,
  `#US`, `#Blob`), method bodies, an IL decoder for every opcode, token
  resolution (MethodDef, Field, MemberRef, TypeDef/TypeRef/TypeSpec).
  Game readers: `enum_values` / `tech_type_names`, `craft_trees` (runs the
  scheme methods on a small value stack, only the instructions they use),
  `tech_data_defaults`.
- `sn-assets::code`: `read_assembly`, `game_code` (`TechDefaults`).
- `sn-inspect code [--trees]`, `sn-inspect code --il <Type> <Method>`
  (console only).
- Test-only encoder (`sn-dotnet/src/encode.rs`) that writes small .NET
  assemblies, for unit tests on synthetic bytes.

**Findings** (all in `docs/formats/dotnet.md`): 21,383 method bodies,
570,477 instructions, 0 errors; 793 `TechType` names (793 distinct
values); 7 menus, 159 nodes (fabricator 100), 134 craft nodes, each with a
TechData entry; 17 TechData defaults (item size 1×1, craft amount 1, craft
time 0, max charge −1, …). `RocketScheme` exists but is never used.

**Dead ends / corrections:**
- First real run stopped at a MemberRef whose parent is a TypeSpec
  (generic `List<…>` constructor); TypeSpecs are now named by their
  generic type.
- `TechData.defaultProperties` matched the `default…` field rule but is
  `new List<string>(defaults.Keys)`, not a value; skipped explicitly.
- The scheme reader needed `newarr`/`dup`/`stelem.ref` (C# `params`
  arrays) and `ret` beyond the planned four patterns; still straight-line
  code, so no stop-and-ask.
- `gameplay.md` said `TechType` had 787 members; the decompiled source has
  793 too, so that was a miscount. Corrected.

**Verified (2026-10-10):**
- `cargo test --workspace` passes without the game (sn-dotnet: 9 tests,
  incl. enum, menu and defaults round trips on our own encoded assembly,
  and every truncation and 3 bit flips per byte without a panic).
- `SUBNAUTICA_DIR=… cargo test -p sn-assets --test real_data -- --ignored
  game_code` passes.
- `cargo run -p sn-inspect -- code` prints the numbers above, exit 0.
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets
  -- -D warnings` clean.

## 2026-10-10 — Lint and format fixes for the Anchor Pod commit

**What:** `cargo fmt` on `crates/sn-assets/src/prefab.rs` (the Anchor Pod
rotation block, line breaks only) and the Anchor Pod real-data test loops
over `expected_rot` with `enumerate` (clippy `needless_range_loop`). No
behaviour change.
**Why:** both came in with f3dc283 and made `cargo fmt --check` and
`cargo clippy -D warnings` fail.
**Verified:** `cargo fmt --all -- --check` and `cargo clippy --workspace
--all-targets -- -D warnings` clean; the Anchor Pod real-data test passes
(`cargo test -p sn-assets --test real_data -- --ignored anchor`).

## 2026-10-09 — P0: gameplay data, headless

**What:** first step of Phase E (`docs/DESIGN.md` § 4.3). New readers:
- `sn-unity::gameplay`: `EntTechData`, `PDAData` (log, databank, scanner,
  `defaultTech`, `analysisTech`, `compoundTech`), `Player` (up to
  `guiHand`, skipping its `GUIStyle`), `Oxygen`, `LiveMixin`,
  `LiveMixinData`, `PlayerMotor` / `UnderwaterMotor`, `PlayerController`,
  `BreakableResource`. `sn-unity::collider`: box, sphere, capsule and mesh
  colliders (for M9a).
- `sn-assets::gameplay`: `tech_data` (`Balance/TechData` JSON, keys the
  game reads, absent keys kept as `None` until P1 reads the code's
  defaults), `ent_tech_data`, `player_data` (the main scene's player
  components and the `PDAData` the player points to),
  `Assets::node_components`.
- `sn-inspect techdata`, `sn-inspect player`, `sn-inspect prefab
  --colliders` (also counts `Pickupable` / `BreakableResource` on placed
  prefabs and on the loot table's prefabs).

**Findings** (all in `docs/formats/gameplay.md`):
- TechData: `{"entries": [...]}`; 463 entries + 513 ingredients = the 976
  `techType` keys of the file search; 0 errors, duplicates or unknown
  keys; 28 ingredient rows (12 tech types) have no entry of their own
  (hypothesis: all-default entries trimmed by the game's editor).
- `PDAData` is not a resource: the main scene's `Player.pdaData` points to
  it. The player and its components are part of the main scene.
- Player: oxygen capacity 45, suffocation 8 s / recovery 4 s, health 100,
  swim speed 7.6 (the scene overrides the code's 6.64), walk 3.5.
- Every collider (24,248 on placed prefabs) and every `BreakableResource`
  (4 prefabs) parses to its last byte.

**Dead end:** my field-listing script took `PlayerController.forwardReference`
(a property) for a field; the exact-end check caught it (4 bytes short).

**Verified (2026-10-09):**
- `cargo test --workspace` (no `SUBNAUTICA_DIR`): all pass, including new
  unit tests on synthetic bytes (`ent_tech_data`, `pda_data`,
  `player_fields`, `oxygen_and_health`, `motors_and_controller`,
  `breakable_resource`, `reads_every_shape`,
  `wrong_class_or_size_is_an_error`, `reads_entries_and_reports_odd_ones`,
  `bad_values_are_errors`); every parser test also checks that each prefix
  is an error and byte changes don't panic.
- `SUBNAUTICA_DIR=… cargo test -p sn-assets --test real_data -- --ignored
  tech_data_and_ent_tech_data player_and_pda_data`: 2 pass.
- `sn-inspect techdata` (0.5 s), `player` (0.8 s), `prefab --colliders`
  (11 s): numbers above.
- `cargo clippy` on the changed crates: clean except a
  `needless_range_loop` in `crates/sn-assets/tests/real_data.rs:953`,
  which comes from the previous commit (Anchor Pod test), not from P0;
  left as it is.
- **Not tested:** any of these numbers in the running game; the GroundMotor
  fields beyond `PlayerMotor`; `Survival`; tech type names (P1).

## 2026-10-09 — Plan: Phase E, the road to a playable game

**What:** `docs/DESIGN.md` § 4.3 (new): the remaining look milestones are
deferred (listed there; their rows unchanged) and Phase E comes first: P0
gameplay data, P1 our own .NET metadata/IL reader (`sn-dotnet`), M9a–M9c
(collision, player, oxygen/health), M10 (multiplayer), M9d–M9f (pick-up and
inventory, crafting, save/load, server-authoritative), then a list of later
blocks toward the full game. New `docs/formats/gameplay.md`: where the game
keeps its gameplay data. The M9 row points to § 4.3.
**Why:** the user wants real gameplay and, in the end, Subnautica 1:1. Most
look milestones are first passes waiting on matched screenshots, and nothing
can be played yet. The user chose (2026-10-09) to read code-only data from
their own `Assembly-CSharp.dll` at runtime rather than store it.
**Findings (decompiled with ilspycmd into the system temp folder, nothing
kept in the repository):** recipes are **not** in code. `TechData` loads the
JSON text asset `Balance/TechData` (found in `resources.assets`: 976
`techType` keys, 40 `craftTime`). Code only: the `CraftTree` menus, the
`TechType` enum names, TechData's defaults.
**Dead end:** the first string search (`CraftData`, `CraftTree`) suggested
recipes were in code; decompiling `CraftData` showed they moved to `TechData`.
**Verified:** docs only, no code changed. **Not tested:** everything in the plan.

## 2026-10-09 — M7g4 (part): the door force fields; extra materials as extra passes

**What:** the user reported the Blood Kelp cache (camera −620 −556 1488) and
its "holographic door" as wrong. The door's force field (`x_Forcefield`)
uses two `UWE/Particles/UBER` materials: `FX_ADDFOG FX_DEFORM FX_MULMAP
FX_SCROLL FX_SOFTEDGES WBOIT` with and without `FX_REFRACTMAP`. Both are
decoded from the compiled programs (`docs/formats/materials.md`
§ `UWE/Particles/UBER` meshes) and drawn by `effects_uber.wgsl`:
- soft edges against the scene depth;
- a scrolling deform map that shifts every texture's uv by
  (map − 1) × `_DeformStrength`;
- a refraction map whose offset goes to the WBOIT target B.yz, which the
  composite already applies.

The variant table and checks are in `objects.rs` (`PARTICLES_VARIANTS`);
draws take four textures (`_MainTex`, `_MainTex2`, `_DeformMap`,
`_RefractMap`). Trap found: both materials hold a float and a vector named
`_RefractStrength`; the program reads the float.

The node has one sub-mesh and two materials. Unity draws extra materials
over the last sub-mesh again; we dropped them. They now draw, and each one
is logged. Others found this way: abandoned base corridors, Precursor
columns, a coral plant's opaque pass, the thermal reactor halo, the Gun's
elevator tube and terminal screen. Nearly all are `MarmosetUBER` or decoded
UBER.

`Update::Material` now boxes its description (clippy: large enum variant).

**Verified:**
- `cargo test --workspace`: 157 pass, including new
  `force_fields_follow_the_decoded_terms` and
  `door_force_field_variants_are_drawn_by_the_effect_pass`.
- `cargo test --workspace -- --ignored`: 19 pass. Clippy and fmt are clean.
- Client at the door (`--start -622 -558 1482 --look -608 -561 1483`):
  - no shader warnings; both portal materials are logged as effect
    materials;
  - effects GPU 0.22 ms;
  - the flat green sheet is now scrolling horizontal streaks with
    refraction and a soft second layer (`out/m7g4/door2.png`, `door3.png`).
- **Not compared with the game on screen.**
- The refraction's y sign is not checked.

**Open concerns** (user review 2026-10-09; the values are the game's, so
they stay as they are; listed in `docs/DESIGN.md` "M7g open"):
1. The spotlight cones are dimmer than in the game screenshot.
2. The Precursor pillars are nearly black; this may be the missing
   atmosphere volume ambient (not checked).
3. The other UBER variants keep the stand-in look.
4. The refraction's y sign is unverified.

## 2026-10-09 — M7h: what the prefabs' placeholders spawn (doors, key terminals, crystals)

**What:** the cache's door, key terminal and the ion crystals on the
pedestals were missing. The game spawns them at run time from
`PrefabPlaceholder`s (`PrefabPlaceholdersGroup.Start`), so they never appear
in the world's cells.
- `sn-unity` `placeholder.rs` reads both scripts. The parsers require the
  data to end at the last field, which checks the layout.
- `sn-assets` attaches the placeholders to their nodes.
- The client spawns each active placeholder's prefab (by its class id's
  world entity info) under the placeholder's parent, nested, creatures
  left out. Every skip is logged with its reason. `--no-placeholders` turns
  this off for comparisons.
- `sn-inspect`: `entities --find TEXT` searches every placement (the
  per-batch listing shows only the first cells), and `prefab KEY --props`
  prints a prefab's material properties.

**Verified:**
- Reader unit tests.
- Real-data test `prefab_placeholders`: the pedestal spawns
  `PrecursorIonCrystal`; the cache root spawns `Precursor_Gun_Terminal2Door`
  and `Precursor_PurpleKeyTerminal`.
- Client at the cache: "placeholders spawned so far: 2091 (189 distinct
  prefabs)"; the door stands in its frame (`out/m7g4/door.png`).
- Frame times are within noise: 8.66 ms with vs 8.24 ms without in one pair
  of runs, 16.92 vs 16.95 ms in a pair under load.
- Not compared with the game on screen.

## 2026-10-09 — M7g4 (part): the Precursor consoles' holograms

**What:** the hologram over the cache console was a solid textured green
funnel; in the game it is a faint funnel with a floating symbol. Its six
materials (`x_Precursor_ComputerTerminal_*`) use three `UWE/Particles/UBER`
variants: `FX_ADDFOG FX_SCROLL WBOIT`, plus `FX_MULMAP` and/or
`FX_FRESNELCLIP`. They are decoded from the compiled programs
(`docs/formats/materials.md`). The colour is 2 × `_Color` × vertex colour ×
the scrolling textures × the day/night strength (by
`_UweLocalLightScalar`). Alpha is the colour's mean brightness × its alpha,
so dark texels are transparent; an optional Fresnel clip applies. Fog and
WBOIT output work as for the glows.
- New `effects_uber.wgsl`, drawn in the same accumulation pass as the
  glows.
- The client draws a UBER material this way only if its keyword set was
  decoded and its blend, depth test and cull match what the pass does.
  Every other material is logged with the reason and keeps the stand-in
  look.

**Dead end (cones):** the cache's spotlight cones looked missing. Six
`Precursor_Cache_Spotlight_Generic_Bright` cones are placed, and debug
renders showed all of them, pointing down from the pillars. Each term of
the glow formula behaves correctly in isolation. They are faint by their
values: `_Color.a` × intensity ≈ 0.08. Measured against the user's game
screenshot, our cones add about +11 to the green channel and the game's
about +28. Our cave is also brighter and bluer than the game's (2, 24, 14),
probably the cache's atmosphere volume ambient, which is not ported. The
rest of the gap is **unexplained**.

**Verified:** client tests of the decoded formula and the variant checks.
The console shows the floating "G" symbol (`out/m7g4/cache-placeholders.png`).
Not compared side by side with the game.

## 2026-10-09 — M7g3 fix: the glows take their light's colour (`VFXVolumetricLight`)

**What:** the first M7g3 build drew the cones as flat grey solids (user
report; `out/m7g3/cone.png`). The shader decode was right (re-checked
against the compiled program term by term); its inputs were not. The
game's `VFXVolumetricLight` script (next to each glow's `Light`) overrides
the material on `Awake`: `_Color` = the light's colour with alpha × light
intensity / 8, plus its own intensity, start offset/falloff, soft edges and
near clip; each `LateUpdate` the glow's renderer is on as the light is. We
used the bare material (white, alpha 1, intensity 0.5).
- `sn-unity` `VfxVolumetricLight` (parser; field layout confirmed on a scene
  copy, 176 bytes), `sn-assets` `VolumetricGlow` on the glow's node (script,
  resolved light, whether `Awake` sets the block; the renderer's enabled
  flag follows the light), the client's `glow_material` (a material per
  distinct set of values).
- Docs: `docs/formats/materials.md` § Fake volumetric lights.

**Verified:** unit test `reads_a_volumetric_light`; real-data test
`volumetric_light_glow` (the ion crystal pedestal: green light (0.42, 1,
0.69) at intensity 3, script intensity 0.35, so `_Color.a` 0.375). Client
at the cone (`--start -1110 -680 -600 --look -1110 -680 -585`): 10
distinct glow values logged (green/teal colours, alpha 0.125–0.75,
intensity 0.175–0.5); the cone is a translucent green beam fading along its
length (`out/m7g3/cone2.png`); at the pedestal the white sphere and cones
are now a faint green haze (`out/m7g3/pedestal3.png`), effects GPU 0.28 ms.
**Not compared with the game on screen.** Not followed: later changes of a
light (day/night or scripts) while playing.

## 2026-10-09 — M7g3: the fake light glows with the game's shader and WBOIT

**What:** `UWE/Particles/WBOIT-FakeVolumetricLight` (the white spheres and
cones: ion crystal pedestals, Precursor spotlights) drawn the way the game
draws it (`docs/DESIGN.md` § 4.2, decided with the user: our own pass after
the fog). Decoded from the compiled D3D11 programs
(`docs/formats/materials.md` § Fake volumetric lights).
- `apps/sn-client/src/effects.rs` (new): the worker sends these parts'
  meshes with vertex colours; the main world spawns `EffectPart`s; the
  render world uploads the meshes once, accumulates the glows into two
  half-float WBOIT targets with the game's blend and weights, then
  composites like `Hidden/WBOIT Composite`, after the sun shafts and
  before tonemapping. `effects.wgsl` (soft edges from the scene's depth,
  Fresnel, near fade, vertex-alpha falloff, water fog at the glow's own
  distance and the fade over its fogged path); `effects_composite.wgsl`.
- `water_common.wgsl`: `water_fog_length` (the in-water fog path, same
  rules as `apply_water_fog`). `sun_shafts.rs`: a `SunShaftsPass` set to
  order after. `objects.rs`: effect materials and meshes, a log line per
  effect material with its values. No new dependencies.
- Decode corrected on re-reading: the fog fade uses the path through
  fogged water past the start distance, not the whole distance; the
  composite's alpha is `(1 − A.a)² + A.a × scene.a`.

**Verified (2026-10-09):** workspace tests 148 pass (4 new formula tests:
the glow's terms, the fog fade's 112.5–125 m, one glow composites as plain
alpha blending, overlapping glows averaged by weight; they check the
formulas in Rust, not the WGSL itself), real-data 17 pass, clippy and fmt
clean. Client at a pedestal (`--start -1121 -686 -712 --look -1125 -688
-705 --benchmark 120 --gpu-timings`): `effects: 21 parts, 3 meshes`, the
effects pass 0.21 ms of GPU time, mean frame 6.99 ms, 0 warnings, and the
glow shader is no longer in the unported list. Screenshots
`out/m7g3/pedestal-final.png` (a soft glow over the pedestal) and
`out/m7g3/cone.png` (a spotlight cone, evenly lit near its apex with hard
sides). Looked into with a debug render of the Fresnel term (≈ 0.9 across
the cone) and logged mesh data: the cone's apex normal is its axis and
its side normals are for a narrower cone, so that look follows from the
game's data under the decoded formula. **Not compared with the game.**

## 2026-10-09 — M7g2: shader names and render state; unported shaders logged

**What:** every material's shader is known by name (`docs/DESIGN.md`
§ 4.2). Nothing is hidden: shaders we haven't ported stay drawn as before
and are logged.
- `sn_unity::Shader` (class 48): the parsed form up to the name:
  properties (with defaults), sub-shaders, passes and their state (blend
  per render target, colour mask, z write/test, cull, offsets, tags); the
  programs' bindings walked over. Layout written from UnityPy's 2019.4 type
  database (`docs/formats/unity.md` § Shaders); 2 tests on synthetic bytes.
- Client: each material's shader name (one parse per shader object); a
  shader other than MarmosetUBER is logged the first time a part with it is
  drawn, and the totals per shader each time loading settles. The UBER
  decision keeps the property fingerprint: it picks exactly the UBER
  shaders (checked on all drawn materials).
- `sn-inspect prefab --materials` uses `Shader` (the string scan is gone),
  tells materials apart by name **and** shader (names repeat across bundles:
  1,975 materials, not 1,973; `Standard` has 4, not 2), prints every shader
  used with its first pass; `unity --all` parses every shader of the game.
- Real-data test `shaders_of_the_fake_volumetric_light`. No new dependencies.

**Verified (2026-10-09):** `unity --all`: 5,472 files, 363 shaders read, 0
unreadable. The 17 shaders of the drawn materials: name, sub-shader and pass
counts, first-pass blend, colour mask, z write, z test, cull identical to
UnityPy (17 of 17; `out/m7g2/oracle.py`). The fake volumetric light is
additive (`Blend One One`), no depth write, no culling. MarmosetUBER's
texture defaults (`_MainTex`, `_SpecTex`, `_Illum` white, `_BumpMap` bump)
read with UnityPy: our defaults match (was a hypothesis). Workspace tests
143 pass, real-data 17 pass, clippy and fmt clean. Client at the lifepod
(`--benchmark 300`): 9 unported shaders logged, totals e.g.
`UWE/Particles/UBER` 91, `WBOIT-FakeVolumetricLight` 35, `UWE/SIG` 33,
`Standard` 27, `SIG Triplanar with Capping` 15 drawn parts; 0 warnings;
mean 11.55 ms (before M7g2: 11.60 ms). At the ion crystal pedestal: 8
shaders logged; the screenshot (`out/m7g2-pedestal.png`) shows a wall in
front of the camera, so no picture of the pedestal yet. **Not compared with
the game.**

## 2026-10-09 — M7g1: the main camera's culling mask (occluder shells gone)

**What:** the client no longer draws renderers on layers the game's main
camera leaves out (`docs/DESIGN.md` § 4.2). The game's rule, no name
matching.
- `sn-unity::Camera` (class 20, up to the culling mask; `draws_layer`),
  `TAG_MAIN_CAMERA`; 2 tests on synthetic bytes.
- `sn-assets::Assets::main_camera` (the `main` scene's enabled camera on an
  active GameObject tagged `MainCamera`, like `Camera.main`); real-data test
  `main_camera_skips_the_occluder_layer`.
- Client: the worker reads the mask at start (logged with the layers it
  leaves out; if it fails, a warning and every layer drawn) and skips such
  nodes in `prefab_parts`, logging each one it skips.
- `sn-inspect scene` uses `Camera`; `prefab --materials` also prints the
  first placement of each material. Plan changed at the user's request:
  shaders not yet ported **stay drawn** and get logged (M7g2), not hidden.
  No new dependencies.

**Verified (2026-10-09):** workspace tests 141 pass; real-data tests 16
pass; clippy `-D warnings`, fmt clean. Client `--start -59 -1192 85 --look
-59 -1199 95 --benchmark 120` (the Lava Castle base): "main camera: culling
mask 0x65ffff17, layers not drawn [3, 5, 6, 7, 25, 27, 28, 31]"; 28 nodes
skipped in the area that loaded, all `Occluder_*_shell` on layer 27
(including the prison aquarium's in the Aurora scene); 0 warnings; 120
frames mean 10.64 ms. The screenshot (`out/m7g1-lavabase.png`) is nearly
black: no sunlight at that depth, and the base's own lights are not drawn
as the game does. **Not compared with the game** on screen.

## 2026-10-09 — Research: occluder shells, white spheres, untextured objects (plan M7g)

**What:** checked an earlier research note (by another model, not in the
repo) on the "invisible walls" in alien bases and the white spheres, and
wrote the plan (`docs/DESIGN.md` § 4.2, M7g1–M7g6). Nothing in the client
changed.
- `sn-inspect prefab --materials`: every material on a drawn node of the
  placed prefabs and the startup scenes, with uses, layers, texture slots,
  render queue, keywords and its shader's name (a heuristic string scan,
  inspection only).
- `sn-inspect scene <name>` also prints each `Camera`'s near/far and
  culling mask (Unity 2019.4 layout, hand-written).
- New `docs/formats/materials.md` (census, per-shader findings);
  `docs/formats/unity.md` § Cameras and layers. No new dependencies.

**Verified (2026-10-09):** `sn-inspect prefab --materials` (65 s): 1,369
prefabs, 0 unreadable, 1,973 materials. `DepthOnly` → `Unlit/DepthOnly`, 39
nodes in 38 prefabs, all on layer 27; `x_AtmoLight_*` →
`UWE/Particles/WBOIT-FakeVolumetricLight`, queue 3101, 68 nodes.
`sn-inspect scene main`: `MainCamera` mask `0x65ffff17` (layer 27 off),
`MainCamera (UI)` `0x20`, `ImguiCamera` `0x80000000`, which matches the
layer names (UI = 5, DebugOverlays = 31). Earlier note **confirmed:** the
occluder shells and the sphere/cone lights are drawn as white meshes by us
and not by the game. **Corrected:** the lights are blended, not opaque
(alpha 1, so they look the same); the door force field is not drawn at all
(its prefab is not placed in the world data). **Missed by it:**
`UWE/Particles/UBER` meshes (1,058 placements) and the triplanar rocks
(976 placements, drawn white). Not checked on screen.

**Housekeeping noticed, not changed:** the repo root holds gitignored
leftovers of earlier sessions (`Voxeland.cs`, `VoxelandChunk.cs` look like
decompiled game code, plus `scratch_*.py`, `shaders.txt`, `test_colors.py`).
They are not tracked, but AGENTS.md wants such output in `out/`.

## 2026-10-09 — M7f3: Lifepod 5

**What:** the `escapepod` scene is placed as a new game does
(`docs/formats/unity.md` § Scenes, Lifepod placement).
- `sn-world::StartMap` (pure: `IsStartPointValid`, `GetRandomStartPoint`
  with our seeded draw, wrap/clamp at the edge; 3 unit tests).
- `sn-unity`: `parse_random_start`, `EscapePod`, `PrefabSpawner` (both
  spawner scripts), `SpawnType` constants; synthetic tests.
- `sn-assets`: `Assets::start_map` (the texture, rows flipped to
  `GetPixel` order), `Scene::place_escape_pod` (returns the player spawn),
  `Scene::follow_targets` (`MoveAndRotateWithTransform` once),
  `Scene::spawns` + `SceneSpawn::placement` (`PrefabSpawnBase.SpawnObj`'s
  rules), `Prefab::set_local`/`world`; unit tests for placement and the
  inverse transform.
- `sn-inspect scene --lifepod [--seed N]`.
- Client: the start point and player spawn are computed before the app
  starts (`--lifepod-seed N`, default 1; `--lifepod X Z`); the worker
  places the pod with the same point and spawns its modules; **the camera
  now starts at the player spawn** unless `--start` is given (before: 0
  −10 0; benchmarks without `--start` are not comparable with earlier
  ones). No new dependencies.

**Verified (2026-10-09):**
1. `sn-inspect scene --lifepod`: start map 0.59 % valid; seed 1 → (−127.73,
   0, −49.75) after 72 draws, seed 2 → (24.92, 0, −133.47); player spawn
   (−126.79, 2.1, −49.75); 6 objects on their targets; 7 spawners fire
   (UI, medical cabinet, 3 power cells, fabricator, radio), their parents
   inside the pod. Two client runs give the same point (seed 1).
2. Real-data test `lifepod_start_and_modules`; all 12 real-data tests of
   sn-assets/sn-unity/sn-world, workspace tests, clippy, fmt: pass.
3. Client, default start: "scene escapepod: 1 top-level objects, 248
   nodes, 25 drawn, Lifepod 5 at …, spawned [6 modules]", 1,242 scene
   entities, 0 warnings, 300 frames mean 11.10 ms. Screenshots
   `out/m7f3-outside.png` (the pod on its float ring at the water line,
   "5 LIFEPOD") and `out/m7f3-inside.png` (seat, wall panel, a module).
   **Not compared** with the game. Known gaps: the camera stands at the
   spawn transform (the game's eye is higher), the interior is lit by the
   outside sky (the pod's own Marmoset sky and lights not used), no
   floating motion, no intro damage effects.

**Differences from the game:** every known gap left by M7f1–M7f3 (Aurora
LODs, explosion timing, culling, effects, skinned animation and blend
shapes, the lifepod's floating, eye height, interior sky and lights, intro
state, module scripts) is listed in the new `docs/DESIGN.md` row **M7f4**;
README "What doesn't work yet" updated to match.

**Also:** commit `16c6542` repaired characters a PowerShell round trip had
re-encoded in `main.rs` and the skinned census (in `9256e34`); edits are
now made with Python or the editor only.

## 2026-10-09 — M7f2: skinned meshes

**What:** skinned meshes are drawn in the pose their hierarchy stores.
- `sn-unity`: `SkinnedMeshRenderer` (renderer fields, mesh, bones, blend
  shape weights, root bone, bounds; `MeshRenderer` shares the renderer
  part); `Mesh::bind_poses`; `MeshGeometry::bone_weights`/`bone_indices`
  from vertex channels 12/13. Synthetic tests for both, cut-short input
  gives errors.
- `sn-assets::skin` (pure: `Σ w · bone · bindPose · v`, normals and
  tangents too, missing bones left out; 4 unit tests on synthetic bones);
  `PrefabNode::bones` (node indices), `Prefab::skinned_geometry`,
  `Prefab::visible` (with indices). Skinned renderers now give the node
  its mesh and materials, so they are drawn (before: skipped, the next
  LOD level shown instead).
- Client: skinned parts are skinned once per prefab node in the worker
  and placed in the prefab root's space; bone-less ones as plain meshes.
- `sn-inspect prefab --skinned` (checks, LOD 0 vs LOD 1 bounds, nearest
  placement); `prefab <KEY>` exports skinned nodes skinned.
  No new dependencies.

**Dead end:** first read the bind poses column by column (assuming
Unity's `m00 m10 …` field order). Sizes still matched LOD 1, as sizes
ignore translation; the bottom-row check (269 of 269 wrong) showed the
file stores them row by row. Fixed before anything used them; the
census now also compares centres.

**Verified (2026-10-09):**
1. `sn-inspect prefab --skinned`: 73 placed prefabs with active skinned
   meshes, 304 renderers drawn: 190 skinned, 113 bone-less (drawn plain),
   1 without bone indices; 0 bones outside their hierarchy; 0 bind poses
   without a 0 0 0 1 bottom row; LOD 0 vs static LOD 1 on the 3 prefabs
   that have both: sizes within 0.1 %, centres within 1 cm. 0 errors.
   Escape pod scene: 31 skinned renderers, all with bones.
2. Real-data test `skinned_lod_matches_its_static_lod`
   (`AbandonedBaseFloatingIsland1`, 1 % tolerance); all 11 real-data
   tests of sn-assets/sn-unity, workspace tests, clippy, fmt: pass.
3. Client at the lifepod start: 16,440 entities, 0 warnings, 300 frames
   mean 12.13 ms (12.27 ms in the M7e2 follow-up run, not A/B in one
   session). Close-up `out/m7f2-braincoral.png` (start 44 −18 96): the
   brain coral (LOD 0 now, bone-less) at its place. **Not compared** with
   the game; no animation (the game's `Animator`s pose these).

## 2026-10-09 — M7f1: scenes, the Aurora

**What:** plan for M7f in `docs/DESIGN.md` (three steps: scenes and the
Aurora, skinned meshes, Lifepod 5). This step: a scene reader and the
scenes the game spawns at start, drawn in the client.
- `sn-unity::scene_scripts`: `MainGameController.additionalScenes`,
  `LightmappedPrefabs.autoloadScenes`, `CrashedShipExploder` (synthetic
  tests, cut-short input gives errors).
- `sn-assets::Scene` (`Assets::scene`, `startup_scenes`, `behaviours`,
  `spawn_lightmapped_prefab`, `swap_aurora_models`); the prefab hierarchy
  reader is shared (`Assets::hierarchy`); `PrefabNode` now has its
  GameObject key and own active flag, `Prefab::set_active` as Unity's
  `SetActive`; `Assets::script_class` public.
- `sn-inspect scene <name> [--tree D | --script Class]`, `scene --startup`.
- Client: the worker loads the startup scenes after the asset index and
  sends each top-level object as an instance, shown always (not streamed);
  the escape pod scene is skipped until M7f3. Spawning one instance is now
  `ObjectStreamer::spawn_instance`, shared by batches and scenes.
  `--no-scenes`, `--aurora intact|exploded` (default intact: a new game).
  No new dependencies.

**Found** (`docs/formats/unity.md` § Scenes): `main` → `Essentials` →
`Cyclops` (template), `EscapePod` and `Aurora` (spawned at the origin).
The Aurora scene also holds four non-streaming world parts (Precursor
prison exterior and aquarium, Lost River base, Lost River large trees).

**Verified (2026-10-09):**
1. `sn-inspect scene --startup`: the chain above; aurora 6 top-level
   objects, intact 330 nodes drawn, exploded 337, 2 objects off / 2 on.
2. Real-data test `startup_scenes_and_aurora` (those numbers, class
   counts 3,189/437/291, escape pod 248 GameObjects, 35 skinned
   renderers); all real-data tests of sn-assets and sn-unity, workspace
   tests, clippy `-D warnings`, fmt: pass.
3. Client at (250 120 650) looking at the Aurora: log "scene aurora: 6
   top-level objects, 3189 nodes, 330 drawn, Aurora intact", 1,196 scene
   entities (1,274 exploded), 0 warnings. 300 frames: 4.69 / 4.75 ms with
   the scenes, 4.47 ms with `--no-scenes`, 5.96 ms exploded (one run each).
   Screenshots `out/m7f1-aurora-far.png`, `out/m7f1-aurora-exploded.png`:
   the ship sits in the water at the right place (by eye). **Not compared**
   with the game; always the most detailed LOD (the game switches to LOD
   1/2 with distance); no fire, smoke, radiation effects.

## 2026-10-09 — M7e2 follow-up: Noisey Wave's sway, UBER grass sky

**What:** after the user's review of M7e (closing two of its open
points; the rest saved as M7e3 and M4b in `docs/DESIGN.md`):
1. Noisey Wave's sway (type 243): read its vertex program again and the
   game's chunk placement (`WorldStreaming/MeshBuilder`: grass vertices
   are in the chunk's space, origin `cellId × 16` at level 0). `sn_terrain::
   build_grass` now keeps each vertex's position in its chunk
   (`chunk_local`); the client puts its part along the material's
   `_ObjectUp` in the second uv set; `grass.wgsl` adds the sway along
   `_WorldWaveDir` with the program's 2D simplex noise.
2. MarmosetUBER grass (4 types) got **no** sky before (Marmoset defaults:
   no ambient), not the global one as written in the M7e2 entry. Checked
   the game: the pooled grass piece (`TerrainPoolManager.chunkGrassPrefab`)
   has no `SkyApplier`, so the global sky is right. `objects.rs`
   `apply_sky` (pulled out of `object_material`, unchanged);
   `TerrainLook::set_global_sky` is called when the objects' worker has
   read the skies and updates the grass materials made and still to make.

**Verified (2026-10-09):**
1. Unit tests: chunk-local positions (corner a multiple of 16, bases at
   the ground); the noise within ±1, smooth and varied; the sway at most
   `_WaveAmount` × height, along the wave direction only; the shader's
   constants; Noisey Wave material values. Workspace tests, clippy, fmt:
   pass.
2. Client at the lifepod: log "4 MarmosetUBER grass materials lit with
   the global sky (found)", 0 warnings or errors, 300 frames mean
   12.27 ms (11.72 ms in the M7e2 entry's run: not compared A/B in
   the same session).
3. Type 243 found with a throwaway probe of the octrees: 4 top voxels in
   batches y 14–19, all in 9-18-12; `sn-inspect grass 9 18 12`: 8 tufts.
   Close-up run there: the shader compiles, no errors. **Not checked by
   eye**: the red grass is hidden in dense seaweed that also sways.

**Dead end:** a first noise test divided by 130 twice and failed; the
noise itself was right (checked by printing values).

## 2026-10-09 — M7e2: the game's grass shaders

**What:** decoded the deferred programs of `UWE/SIG Terrain Grass`,
`UWE/SIG` and `UWE/SIG AlphaCutout + Noisey Wave`
(`docs/formats/terrain-materials.md` § Grass shaders) and ported them:
`grass_look.rs` + `grass.wgsl` (vertex waving from the vertex colours,
world-space mask, top/bottom tints with the height gradient, cutoff, SIG
specular/gloss/glow, the game's light pass). `sn-assets::GrassLook` now
knows its shader (`GrassShader`, by properties), the SIG, mask, specular
and glow maps and the parsed material. MarmosetUBER grass (4 types) is
drawn with our object shader: `objects.rs` `material_desc` (pulled out of
the worker's material loading, unchanged) and `object_material` shared.
Not ported: Noisey Wave's sway (type 243; ≤ 8 cm, needs the game's chunk
origin), biome skies for UBER grass (global sky).

**Verified (2026-10-09):**
1. Real-data test `terrain_materials_resolve_and_decode`: shader per
   grass type as UnityPy names them (48 Terrain Grass, 6 + 1 `UWE/SIG`,
   1 Noisey Wave, 4 MarmosetUBER); textures 287 = 211 terrain + 76 grass.
   All 11 real-data tests of the touched crates, unit tests (3 new: wave
   formula and shader constants, material values), clippy, fmt: pass.
2. The objects are unchanged by the refactor: lifepod census before and
   after equal (1,531/1,567 UBER materials, 16,216 entities, 1,619
   materials, 0 warnings).
3. Benchmarks with / without grass: lifepod 11.72 / 11.48 ms; Grassy
   Plateaus 9.25 / 8.97 ms; lifepod at midnight 11.84 / 11.44 ms. Log:
   60 grass materials, 56 with the grass shader, 4 with the object shader.
4. Waving: two runs of the same view at different times differ in 9.5 %
   of the seaweed region's pixels with grass, 2.9 % without (sand: 13.1 /
   14.4 %, the caustics). Seaweed region mean RGB 87.7/85.5/83.1 (first
   pass) → 85.4/85.9/84.3 (darker bases). Screenshots
   `out/m7e2-start.png`, `out/m7e2-plateaus.png`, `out/m7e2-night.png`.
   **Not compared** with the game.

**Dead ends:** the keyword lists our disassembly script prints next to
each program can belong to a neighbour (as noted in M8c7); the `UWE/SIG`
variant with the SIG map was found by its constant table instead. A
first script edit of `objects.rs` stopped half-way (an anchor changed by
rustfmt) and was finished by hand; the census above checks the result.

## 2026-10-09 — M7e1: terrain grass

**What:** the 60 block types with grass now grow it, placed by the game's
rules (`docs/formats/terrain-materials.md` § Grass): 4 spots per face,
tilt range, density or Perlin noise, jitter, spin, Z-up turn, scale,
the per-chunk budget of 10,000 vertices/triangles with the raised
reduction, vertex colour/height. New: `sn-unity::GrassSettings` (all of
`VoxelandBlockType`'s grass fields), `sn-assets::TerrainGrass` /
`GrassLook` (mesh geometry, textures, material values) and
`TerrainMaterials::grass_types`, `sn-terrain::build_grass` (pure, 7 unit
tests), `sn-inspect grass X Y Z` and a grass census in `terrain-materials`.
The client builds one merged mesh per (batch, type) in the terrain worker
at level of detail 0 and draws it with a first-pass object material, no
shadow casting; `--no-grass`. Dependency: `sn-assets` now depends on
`sn-terrain` (ours; for `grass_types`).

**Verified (2026-10-09):**
1. `sn-inspect grass 12 18 12` twice: 7,826 tufts, 608,726 vertices,
   752,923 triangles, same hash `73e29a516d24d7e8`.
2. Real-data test `terrain_materials_resolve_and_decode` (updated): 60
   grass types; textures 261 = 211 terrain + 50 grass-only; mesh sizes of
   `coral_reef_grass_03` (18/18) and `Coral_reef_small_deco_07` (125/516)
   equal UnityPy's. All 11 real-data tests of the touched crates, unit
   tests, clippy, fmt: pass.
3. Benchmarks (two runs with grass, one without): lifepod 34,469 tufts,
   2,891,017 triangles in 8 batches; 11.62 / 11.68 ms vs 11.46 ms
   without. Grassy Plateaus (−700 −94 −300): 41,007 tufts, 1,015,413
   triangles; 9.09 / 9.04 ms vs 8.80 ms. Counts equal on both runs.
   Screenshots `out/m7e-start.png` (green coral grass on the plateau tops
   and the sand), `out/m7e-plateaus.png` vs `m7e-plateaus-nograss.png`
   (the red seaweed fields). **Not compared** with the game; the grass
   stops ~100 m out (the game also draws its level 1 with reduction 0.5).

**Found:** four grass types (52, 76, 83, 251) use coral materials with
MarmosetUBER keywords (`_Cutoff` 0), not the grass shader; noted for M7e2.

## 2026-10-09 — M7d: spawn slots filled

**What:** the cells' 90,289 `EntitySlotsPlaceholder`s are filled the way
the game fills them when a cell first loads (`docs/formats/entities.md`
§ Spawn slots). New: `sn-world::slots` (slot parse, the game's choice
rule, copies within 4 m, Z-up turn, our seeded SplitMix64 keyed by
placeholder id and slot index), `sn-unity::json` (the catalog's JSON
reader moved out of `addressables.rs`, now accepting `//` and `/* */`
comments), `sn-unity::parse_world_entity_data`, `sn-assets::loot_table` /
`entity_infos`, `sn-inspect slots [X Y Z] [--seed N]`. The client spawns
the fillers with the cells at their own cell level (creatures skipped, as
for placed objects); `--slot-seed N` (default 1), `--no-slots`. No new
dependencies.

**Finding:** the roadmap's hypothesis was wrong: slots hold **no
vegetation**. Seed 1: 141,639 objects, of which 102,777 creatures (not
drawn), 35,808 resource outcrops (limestone, quartz, …), 1,186 eggs, 1,029
fragments. The missing small plants must come from somewhere else (M7e
grass is the next candidate).

**Verified (2026-10-09):**
1. `sn-inspect slots`: 90,289 placeholders, 1,288,139 slots, 0 errors;
   190 distribution entries / 1,295 rows / 352 biomes, 3,336 entity infos,
   every pickable prefab has an info and a path; all but 7 slots inside
   their batch; counts per biome logged (e.g. Safe Shallows sand flat
   4,766 slots, 559 filled, 830 objects). Hash of all spawns
   `0d4ea805ed9120b2` on two runs with seed 1, `8edc2ef9122ad69a` with
   seed 2.
2. Real-data test `spawn_slot_tables_and_fill` (lifepod batch: 30
   placeholders, 1,719 slots, 325 objects at seed 1, 334 at seed 2; same
   seed equal) and unit tests (slot parse and corruption, choice rule,
   copies/rotation, determinism, JSON comments, WorldEntityData) pass;
   all real-data tests of sn-unity/sn-assets/sn-install pass (catalog with
   the moved JSON reader). Clippy, fmt: clean.
3. Lifepod benchmark (300 frames, two runs with slots, one without): 328
   slot objects shown (478 more entities: 16,216 vs 15,738), identical on
   both runs, 0 warnings; mean 11.10 / 11.20 ms vs 11.02 ms without.
   Screenshots `out/m7d-slots.png` / `m7d-noslots.png` (wreck debris
   appears on the sand), `out/m7d-outcrops-slots.png` /
   `m7d-outcrops-noslots.png` at (−76 −7 −100): a limestone outcrop on
   the slope, seated on the terrain. **Not compared** with the game.

**Dead ends:** UnityPy can't read `WorldEntityData` by type tree (no
script types); its layout was read from the raw bytes (68 bytes per info).

## 2026-10-09 — M8e3: the objects' lights light the world

**What:** placed objects' realtime lights spawned with them
(`objects.rs` `spawn_light`; Bevy lights only for culling and
clustering) and applied by `game_local_lights` in `game_light.wgsl` with
the game's point/spot formula (falloff curve **hypothesis**, see
`docs/formats/lighting.md` § How we render local lights); objects'
directional lights (≤ 8) through the light parameters; `--no-local-lights`
for comparisons. `sn-inspect prefab --lights` now also lists the densest
50 m columns of lights (all, and glowing coral only) with the prefab
holding most of them.

**Verified (2026-10-09):** unit tests `light_falloff_at_known_distances`
(range 10 m: 1 at 0 m, 0.5 at 2 m, 1/17 at 8 m, 0 at 10 m; continuous,
never rising) and `shader_uses_the_same_falloff` (the shader holds the
same constants). Night benchmarks, lights on / off: lifepod 458 point + 4
directional lights, opaque pass 1.96 / 2.00 ms, image mean RGB 8.4 13.6
16.7 / 8.2 13.1 16.2 (`out/m8e3-night-*.png`); Grand Reef glowing coral
(−1325 −500 −370) 1,744 point + 7 spot lights, opaque 1.11 / 0.86 ms,
mean 0.0 4.4 7.0 / 0.0 4.0 6.1 (`out/m8e3-reef-*.png`: the rock around
the coral lit). **Not compared** with matched game screenshots.

**Dead ends:** the densest light columns of the whole world are all
Precursor interiors (gun, prison, lava castle); a first "dense area"
render at (425 −55 1095) put the camera inside the mountain (black
image). The glowing-coral-only list was added for that.

## 2026-10-08 — M8e1: the game's lights read; sun intensity units fixed

**What:** `sn-unity::Light` (Unity 2019.4 layout, 264 bytes) and
`DayNightLight`; `PrefabNode::lights` / `day_night_light`;
`sn-inspect prefab --lights` (census of every placed prefab). Decoded the
point and spot light programs (`docs/formats/lighting.md` § Point and spot
light passes). Found `GraphicsSettings.m_LightsUseLinearIntensity` off:
lights are `linear(colour × intensity)` in Unity's passes; our sun in the
surface pass was `linear(colour) × intensity` → new `SkyState::sun_light`
(fog, shafts and water keep the script value, as the game).

**Verified (2026-10-08):** synthetic tests (Light, truncation); real-data
test `prefab_lights` (a glowing kelp prefab: 4 point lights, intensity 1.5,
range 15, colour as UnityPy; the safe shallows "Bounce" light and its day
curves). Census: 226 of 1,369 placed prefabs carry lights; ~9,970 lights
in the world's placements (9,186 point, 561 spot, 123 directional, 99 with
shadows). Sun in the light pass 0.886 → 0.780 (red) at 09:36; sunlit sand
3.5 % darker, shadowed sand unchanged (`out/m8e-sunfix.png`). Tests pass.

**Dead ends:** a first benchmark after the change saved no new screenshot
(an earlier client was still running and held the exe); the comparison
used the old image and showed "no change" — redone.

## 2026-10-08 — M8c7 (first pass): object specular, glow and biome skies

**What:** decoded MarmosetUBER's deferred pass and the Marmoset sky
classes (`docs/formats/lighting.md` § Objects). New: `sn-unity::marmo`
(`MarmoSky`, `SkyApplier`, `MarmoSkiesPrefabs`, unit tests on synthetic
bytes), `sn-assets::marmo_skies` (biome skies + global sky),
`PrefabNode::sky_applier`. The client's object shader now ports the UBER
G-buffer: specular from `_SpecTex`/`_SpecColor`/`_SpecInt`/`_Fresnel`, gloss
from `_Shininess`, glow from `_Illum` with day/night strengths
(`_UweLocalLightScalar`, new texel 11 of the light parameters), the sky's
camera exposure, SH ambient and unlit flag; no Unity ambient on UBER
objects (it was added before; the shader doesn't). Materials are made per
(material, sky); the sky is picked per placed object from its biome.

**Verified (2026-10-08):** real-data test `marmo_skies_and_sky_appliers`:
37 skies for 145 biomes, values equal to the UnityPy readout. Benchmark at
the lifepod: 1,537 UBER materials (1,482 specular maps, 559 glow maps),
materials made for 13 skies; `out/m8c7-start.png` vs `m8c6-start.png`:
terrain sand unchanged (mean RGB 142/179/164 vs 141/177/162), the floating
boulder darker (58/77/68 vs 79/95/88: no Unity ambient), glowing grass
tips greener. Frame time 13.2–16.6 ms over three runs (11.1 ms last
round), but runs vary a lot right now (shafts 1.9 → 2.7 ms GPU with no
change to them); **not A/B tested**. Tests, clippy, fmt: pass. **Not
compared** with matched game screenshots.

**Dead ends:** the shader-index keywords from the program's preceding bytes
were wrong (they belong to neighbouring programs); variants were matched by
diffing bytecode instead.

## 2026-10-08 — M8c6 (first pass): sun shadows, shadowed light shafts

**What:** the game's shadow settings read (`QualitySettings` High: 4
cascades over 50 m, soft; `docs/formats/lighting.md` § Sun shadows). The
sun now casts Bevy shadows with these cascade bounds and Gaussian
filtering; the game lighting already used them. Terrain and objects beyond
the nearest level of detail don't cast (`NotShadowCaster`, updated when a
batch changes level). The light shafts now sample the shadow map through
Bevy's view bind group (the pattern of Bevy's volumetric fog), one
comparison per step.

**Verified (2026-10-08):** `out/m8c6-start.png` (the floating boulder's
shadow on the sand), `out/m8c6-shafts.png` (shafts cut into beams).
Benchmark at the lifepod: 11.1 ms mean (21 ms before limiting the
casters; 9.5 ms without shadows). Shafts 1.9 ms GPU. Tests, clippy, fmt:
pass.

**Dead ends:** shadows from every entity cost ~11 ms of CPU (Bevy prepares
shadow draws per entity and cascade); the soft filter in the shafts' 200
samples per pixel cost 4.8 ms.

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
