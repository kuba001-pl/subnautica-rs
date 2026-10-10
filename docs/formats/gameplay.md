# Gameplay data

Where Subnautica (build 10) keeps the data its gameplay needs, so that we
can read it from the player's install at runtime. Written for Phase E
(`docs/DESIGN.md` § 4.3). Found by reading the game's code with a decompiler
(output kept in the system temp folder, never in the repository), by
searching the install's files, and (P0) by reading the data with our own
code. No game content is reproduced here; numbers are counts or values
our tools print.

Readers: `sn-assets::gameplay` (`tech_data`, `ent_tech_data`,
`player_data`), `sn-unity::gameplay` and `sn-unity::collider` (the object
layouts). Tools: `sn-inspect techdata`, `sn-inspect player`,
`sn-inspect prefab --colliders`.

## Serialized scripts: how the layouts were checked

The game's files have no type trees, so every script's layout is taken
from the field declarations in the decompiled class (declaration order;
base class fields first; only `public` or `[SerializeField]` fields that
are not `[NonSerialized]`, `static`, `const` or properties):
- `bool`: one byte, then aligned to 4. Enums: `i32`. Strings and arrays:
  `i32` length, then the data, then aligned to 4. Object references: PPtr
  (`i32` file, `i64` path id). `[Serializable]` classes and structs:
  their fields inline.
- **Confirmed (real data, P0):** every parser that reads all fields
  (`PDAData`, `EntTechData`, `Oxygen`, `LiveMixinData`, `UnderwaterMotor`,
  `PlayerController`, `BreakableResource`, the four colliders) ends exactly
  on the object's last byte. Parsers that stop early (`Player`,
  `LiveMixin`, `GroundMotor`'s base fields) say so in the code.
- Trap: the decompiler shows `PlayerController.forwardReference` like a
  field, but it is a property; it is not in the data.

## Recipes and item data: `Balance/TechData`

- **Confirmed (code):** `TechData.Initialize` loads the text asset at the
  resources path `Balance/TechData` and parses it as JSON (LitJson). The
  per-entry keys it reads are `techType`, `itemSize` (`x`, `y`),
  `backgroundType`, `equipmentType`, `slotType`, `craftTime`,
  `craftAmount`, `ingredients` (each `techType`, `amount`), `linkedItems`
  (tech types), `processed`, `buildable`, `soundPickup`, `soundDrop`,
  `soundUse`, `harvestType`, `harvestOutput`, `harvestFinalCutBonus`,
  `maxCharge`, `energyCost`, `poweredPrefab`.
- **Confirmed (code + parser):** the top level is an object with an
  `entries` array (`TechData.Deserialize`); entries without `techType` are
  skipped; tech types are added with `Dictionary.Add` (a duplicate would
  throw).
- **Confirmed (parser, real-data test `tech_data_and_ent_tech_data`):**
  463 entries, 0 without `techType`, 0 duplicates, 0 keys the game does
  not read. 463 entries + 513 ingredients = 976, the number of `techType`
  keys found by the earlier file search; 40 `craftTime` (as found). 245
  entries have a recipe; craft times 2–10 s.
- **Confirmed (parser):** 28 ingredient rows (12 tech types) name a tech
  type that has no entry of its own. **Hypothesis:** these are items whose
  fields are all defaults, which the game's editor trims
  (`TechData.TrimDefaults`), so the game uses the defaults for them.
- **Confirmed (code):** keys missing from an entry fall back to defaults
  written in the code (`TechData.defaults`), not in the JSON. Our reader
  keeps them as "absent"; P1 reads the defaults from the DLL
  (`sn-assets::game_code`, see `dotnet.md`).
- **Confirmed (P1, real data):** all 12 tech types used as ingredients
  without an entry of their own have a `TechType` name. Whether all their
  fields really are defaults (the hypothesis above) is not checked.

## Prefab ↔ tech type: `EntTechData`

- **Confirmed (code):** `CraftData` loads the resource `EntTechData`, a
  ScriptableObject with `Entry[] entTechMap`, each `string prefabName,
  TechType techType`.
- **Confirmed (parser):** 703 entries, 703 distinct prefab names (all
  lower case, as `CompileTimeCheck` requires), 548 distinct tech types, of
  which 428 have a TechData entry; no entry has tech type 0.

## Unlocks: `PDAData`

- **Confirmed (code):** the `Player` script holds a reference `pdaData`;
  `PDAData.Initialize` passes it to `PDALog`, `PDAEncyclopedia`,
  `PDAScanner` and `KnownTech`. Its fields: `defaultLogIcon`, `log`,
  `encyclopedia`, `scanner`, `defaultTech`, `analysisTech`,
  `compoundTech`.
- **Confirmed (parser):** reached from the main scene's `Player` (its
  `pdaData` PPtr points into another file; the object it resolves to has
  the script class `PDAData`). 179 log entries, 323 databank entries, 268
  scanner entries (53 fragments, 3 locked), 46 starting blueprints
  (`defaultTech`), 126 `analysisTech` (86 unlocks, 3 story goals), 3
  `compoundTech` (5 dependencies). Every starting blueprint has a TechData
  entry.

## Player numbers (the `main` scene)

- **Confirmed (scene census):** the `main` scene holds one `Player`,
  `Oxygen`, `OxygenManager`, `PlayerController`, `UnderwaterMotor` and
  `GroundMotor`; the player is part of the scene, not a separate prefab.
- **Confirmed (parser; values are those `sn-inspect player` prints, not
  checked in the running game):**
  - `Player`: `playerSphereRadius` 0.5, `suffocationTime` 8 s,
    `suffocationRecoveryTime` 4 s, `crushDepth` 0, 3 equipment slots. Its
    `textStyle` is a built-in `GUIStyle`; its layout (name, 8 states of
    background PPtr + colour, 4 rect offsets, font PPtr, size, style,
    alignment, two bools, clipping, image position, content offset, fixed
    width and height, two bools) was worked out from the bytes: the fields
    after it land on the code's initial values (8 s, 4 s).
  - `Oxygen` with `isPlayer`: `oxygenCapacity` 45.
  - `LiveMixin`: `health` 100; its `LiveMixinData`: `maxHealth` 100.
  - `PlayerController` (it overwrites the motors' speeds when the motor
    mode changes, `SetMotorMode`): swim max speed 7.6 forward, backward
    and sideways, 6.3 vertical, acceleration 20; Seaglide 15 / 11.8 /
    11.8 / 7.9, acceleration 36.56, drag 2.5; walk/run 3.5 forward, 5
    backward and sideways; stand height 1.5, swim height 0.5, camera
    offset −0.25, radius 0.3. The swim speeds differ from the code's
    initial values (6.64), so the scene's values are the ones that count.
  - `UnderwaterMotor` and `GroundMotor` (`PlayerMotor` fields): gravity 12,
    drag (swim 2.5 / 2), accelerations (water 20, ground 45, air 5), jump
    height 2.
- `GroundMotor`'s own fields: § Player movement (M9b).
- Not read yet: `Survival` (food and water). The `Player` fields after
  `guiHand`: read in M9g1 (§ The player's body). `OxygenManager` has no serialized number (its rate is code
  only, § Oxygen, health and death).

## Colliders (on placed prefabs)

- **Confirmed (parser, `sn-inspect prefab --colliders`):** every collider
  starts with `m_GameObject`, `m_Material`, `m_IsTrigger`, `m_Enabled`
  (two bytes, aligned to 4), then: box `m_Size`, `m_Center`; sphere
  `m_Radius`, `m_Center`; capsule `m_Radius`, `m_Height`, `m_Direction`,
  `m_Center`; mesh `m_Convex` (aligned), `m_CookingOptions`, `m_Mesh`.
  All 24,248 collider components of the 1,369 placed prefabs parse to
  their last byte.
- Census (components / triggers / prefabs / placements in the world):
  box 21,059 / 579 / 781 / 168,789; sphere 460 / 136 / 192 / 45,063;
  capsule 2,445 / 33 / 226 / 57,320; mesh 284 / 0 / 15 / 438 (1 convex,
  no null mesh). 345 placed prefabs have no collider.

## Collision (M9a)

- **Confirmed (decompiled code + `clipmaps-high.json`):** terrain collides
  only at the finest clipmap level. Level 0 of `clipmaps-high.json` has
  `colliders: true` and 7×7×7 chunks of `chunkMeshRes` 16 voxels, so
  about ±56 m around the player; levels 1–4 have `colliders: false`. Each
  chunk's collision mesh (`VoxelandCollisionMeshSimplifier.Build`) takes
  corners 0, 2, 4, 6 of each visible face as two triangles (0-2-4,
  0-4-6). Above 100 triangles and 100 vertices it calls a native plugin,
  `SimplifyMeshPlugin.SimplifyMesh(0.8, 0, …)`, with chunk-border vertices
  fixed. We don't port that plugin: our collision uses the unsimplified
  level 0 surface. The collider is put on layer 30.
- **Confirmed (decompiled code):** under water the player is a
  `Rigidbody` with a `CapsuleCollider` (`UnderwaterMotor`). Radius =
  `PlayerController.controllerRadius`; height = `swimheight −
  cameraOffset`; centre y = `−height / 2 − cameraOffset`. With the scene's
  values (0.3, 0.5, −0.25) that is radius 0.3, height 0.75, centre
  0.125 m below the camera. Standing uses `standheight − cameraOffset` =
  1.75 (`GroundMotor`, M9b).
- **Confirmed (real-data test `lifepod_colliders`):** the escape pod scene
  has 40 colliders the player can hit, all boxes, on layer 0. The reader
  also left out 10 triggers, 1 disabled collider and 3 on inactive nodes.
- **Hypothesis (Unity's documentation, not checked in the game):** how
  colliders scale. A box scales per axis. A sphere's radius scales by the
  largest |scale|. A capsule's radius scales by the larger |scale| of its
  two cross axes, and its height by its own axis.
- The physics layer collision matrix and which side of a triangle
  blocks: § Player movement (M9b).

## Player movement (M9b)

- **Confirmed (parser, real-data test `player_movement_data`, `sn-inspect
  player`):** `GroundMotor`'s own fields (Unity's `CharacterMotor`
  classes `CharacterMotorMovement`, `…Jumping`, `…MovingPlatform`,
  `…Sliding`, then the controller's): step offset 0.4 m, slope limit 60°,
  max fall speed 50, sliding speed 7 (sideways control 1, speed control
  0.2), jumping enabled with base height 1 and extra height 4.1, moving
  platform transfer 2. Its own max speeds (10) are overwritten by
  `PlayerController.SetMotorMode` (*confirmed*, decompiled code), so the
  walk speeds are `PlayerController`'s (3.5 / 5).
- **Confirmed (same test):** the player's `Rigidbody`: mass 70, drag 2.5,
  no gravity (`UnderwaterMotor` applies its own), rotation frozen
  (constraints 112), continuous collision detection. Player layer 19
  ("Player"). The ocean level is the `Ocean` object's y: 0.
- **Confirmed (`globalgamemanagers`, same test):** `TimeManager` fixed
  time step 0.02 s (our player steps at 50 Hz); `PhysicsManager` gravity
  −9.8 (the motors use their own 12), `queriesHitBackfaces` false. The
  layer collision matrix: layer 19 collides with every layer except 9
  ("OnlyVehicle"). `TagManager`'s sorting layers are a name and an id
  each (the editor-only `locked` flag is not stored).
- **Counted (`sn-inspect walk`, `sn-inspect swim` seeds 1–5):** every
  solid collider loaded in these runs (110–137 per run, counted once per
  prefab) is on layer 0, so the layer filter drops none of them yet. The
  terrain's colliders are on layer 30, which the player collides with.
- **Confirmed (real-data test):** the lifepod has 8 cinematic hand
  triggers (`CinematicModeTrigger` + `PlayerCinematicController`), each
  with an end point: 2 enter the pod (`BoardEscapePod`), 6 leave it.
  4 end points are marked VR-only (`onlyUseEndTransformInVr`); we use
  them anyway, because without the animation the end point is the only
  place we have. In a new game the two first-use triggers are active and
  their normal twins not (`EscapePodFirstUse`); the first use swaps them.
  The pod's `UseableDiveHatch` sits on an inactive node: the triggers,
  not the dive hatch, move the player in and out.
- **Confirmed (PhysX source, read only; Unity 2019.4.36f1, from
  `globalgamemanagers`, ships PhysX 4.1): triangle-mesh colliders are
  one-sided.** The front is the side `(b − a) × (c − a)` points to, on
  the mesh's own vertex order (Unity's front face). Three paths do this:
  - Rigid-body contacts (the swimming capsule) skip a triangle when the
    capsule's centre is behind its plane. That holds both for PCM
    contacts (`GuPCMContactConvexCommon.cpp`, "Backface culling"), which
    are Unity's default, and for the older path
    (`GuContactCapsuleMesh.cpp`).
  - Mesh sweeps cull a triangle the sweep moves along the normal of,
    unless the mesh is double-sided or the query asks for both sides
    (`GuSweepsMesh.cpp`). The character controller (walking) sweeps with
    the default flags, both-sides commented out
    (`CctCharacterController.cpp`).
  - Raycasts skip back faces because `queriesHitBackfaces` is false.

  A mirroring scale flips the winding back (`flipsNormal`).
  **Hypothesis:** Unity doesn't mark mesh colliders double-sided (2019.4
  has no such option) and hands PhysX the indices unchanged. This is not
  checked in the engine binary. Our `sn-sim` follows these rules.
- **Measured (`sn-inspect prefab --winding`):** our triangles face the
  same way. Terrain, rays straight down over 4 shallow batches: 17,066
  first hits on a front face, 1 on a back face. Mesh colliders of the 14
  placed prefabs that have them, rays from outside along 6 axes: 6,012
  front, 152 back. 147 of the backs are `Precursor_Aquarium_Sand_Drift`,
  an open sheet seen from below. Over deep batches, rays from above start
  inside rock and see the surface from behind, so only shallow batches
  count. Our terrain mesher's faces point out of the rock (`sn-mesh` test
  `sphere_is_closed_oriented_and_faces_out`). `voxel_to_world` only
  translates, so terrain fronts face the water, as the game's do.

## Oxygen, health and death (M9c)

Read in the decompiled code; the numbers from the scene and the DLL
(`sn-inspect player`, real-data test `vitals_and_look_data`). The rules
are ported in `sn-sim::vitals`.

- **Confirmed (code):** "under water" for breathing is
  `Player.UpdateIsUnderwater`, not M9b's swimming flag: never in the
  lifepod (`escapePod`), else the transform below the ocean level. With
  no sub or vehicle, `CanBreathe` is "not under water".
- **Confirmed (code):** depth classes (`Player.GetDepthClass`, the player
  has no `CrushDamage`): depth = max(0, level − y); > 200 m crush, > 100 m
  unsafe, > 0.1 m (`GetSurfaceDepth`) safe, else surface. Breath period
  (`GetBreathPeriod`): 3 / 2.25 / 1.5 s, surface 99999 s. Oxygen per
  breath (`GetOxygenPerBreath`): period × 1 / 1.5 / 2 (no rebreather, not
  piloting, Survival mode). So 1, 1.5 and 2 units per second.
- **Confirmed (code):** a breath happens in `Player.Update` when the
  player cannot breathe and stats are not frozen, each time `Time.time`
  crosses a multiple of the period (`ScalarMonitor.DidChangeInterval`
  over game time, not time under water). So the first breath after
  going under comes after 0 to one period.
- **Confirmed (code + IL):** `OxygenManager.Update` adds
  `oxygenUnitsPerSecondSurface` × `deltaTime` when a source's object is
  above level − 1 m or the player can breathe, unless a cinematic plays
  or the player is in a water park. The field is private, set by its
  initialiser in the constructor: **30** (read from the DLL). The
  player's `Oxygen` is on the `Player` object itself (0 m above it,
  scene).
- **Confirmed (code):** `Utils.NearlyEqual(x, 0)` is true only for x = 0
  exactly (its small-number branch compares with `epsilon ×
  float.MinValue`, a negative number), so suffocation starts only at
  exactly 0 oxygen.
- **Confirmed (code):** suffocation (`SuffocationUpdate`, a `Sequence`
  starting at t = 1): at 0 oxygen, t runs to 0 over `suffocationTime`
  (8 s, scene); the update after it gets there kills the player. Oxygen
  back: t runs to 1 over `suffocationRecoveryTime` (4 s), then a reset.
  The screen overlay 0 gets 1 − t.
- **Confirmed (code):** `LiveMixin.TakeDamage` × `DamageSystem
  .damageMultiplier` (1); death at health 0 (`Kill`, then `OnKill`). The
  only player damage without creatures: `Player.OnLand` from the walking
  motor, out of the water: (−min(0, impact y + 10)) × 2.5, impact = the
  velocity at the start of the step (`GroundMotor.previousVelocity`).
  `Player.CrushDamageUpdate` exists but nothing calls it.
- **Confirmed (code):** death and respawn (`OnKill`,
  `ResetPlayerOnDeath`): input, the player controller and the mouse look
  off, stats frozen; after 5 s the player goes to the respawn point (no
  sub: the last lifepod's `playerSpawn`, in the pod); after 1 s more and
  the world settled, `ResetHealth` then the respawn event's
  `LiveMixin.OnRespawn` (health = `maxHealth` × `startHealthPercent`),
  oxygen full, suffocation reset; 1 s later stats unfrozen and input
  back. The inventory is lost (`Inventory.LoseItems`; no inventory yet).
- **Confirmed (parser, real data):** `LiveMixin`'s serialized fields are
  `data`, `health`, `startHealthPercent`, `damageClip`, `deathClip`,
  `player`; the data ends there (its two `Event<float>` fields are not
  serialized). The player's: health 100, `startHealthPercent` 1.
- **Measured (`sn-inspect dive`, seeds 1–5):** oxygen 0 at 42.26 s
  after going under (the first breath 0.22 s after: phase of the clock),
  death 8.02 s later, respawn 5.02 s, restore 1.02 s, controls 1.02 s
  (the 0.02 s is our one physics step). Oxygen stays within one breath
  (3 units) of a steady 1 unit/s drain. Refill from 23–29 units to 45 in
  0.56–0.74 s after surfacing.

## Mouse look (M9c)

- **Confirmed (code):** `MainCameraControl.OnUpdate` in normal play adds
  the look delta to `rotationX` (yaw, unbounded: `minimumX` and
  `maximumX` are never used) and `rotationY` (pitch, up positive),
  clamped to `minimumY`…`maximumY`. The camera's pitch is −`rotationY`,
  split between `cameraUPTransform` (looking up) and the camera's own
  transform (looking down). There is no smoothing of the look. Camera
  bob, strafe tilt, impact bob and shake are added on top (not ported:
  "After Phase E" item 3).
- **Confirmed (code):** `GameInputSystem.GetVector2(Look)` for a mouse
  (`<Mouse>/delta`, a `Delta` control): delta × `MouseSensitivity` × 1.5 ×
  0.5, y negated if `InvertMouse`. The default sensitivity is the `const`
  `defaultMouseSensitivity`: **0.15** (read from the DLL), so 0.1125° per
  unit of mouse delta. The player's own settings are saved by the game
  (`InputSystem/MouseSensitivity`, `InvertMouse`); we don't read them.
- **Confirmed (parser, real data):** `MainCameraControl` is in the main
  scene on `camRoot`, at the player's origin; all its fields read to the
  last byte: `minimumY` −87, `maximumY` 87 (the code's initial values are
  ±80, so the scene's count), `skin` 0 (the camera sits at its parent),
  `cameraTiltMod` 0.1, `camPDAZOffset` 0.18.
- **Hypothesis:** Unity's Input System reads the mouse delta on Windows
  as raw input counts, as Bevy's `AccumulatedMouseMotion` does, so the
  same physical movement turns the view by the same angle. Not checked
  side by side.

## The player's body (M9g1)

Reader: `sn_unity::PlayerFields` (now every field), `ArmsController`;
`Assets::player_body` (`crates/sn-assets/src/player_body.rs`). Tool:
`sn-inspect player --body`. Test: `real_data.rs::player_body`.

- **Confirmed (parser, real data):** `Player` reads to its last byte. After
  `guiHand` come `infectedMixin`, two `AnimationCurve`s
  (`infectionRevealCurve`, `infectionCureCurve`), `infectionRevealSound`
  and `leftHandBone`. Its `Event<T>` and `MonitoredValue<T>` fields are
  generic classes, which Unity 2019.4 does not serialize, so they are not
  in the data. `ArmsController` reads to its last byte too.
- **Confirmed (scene, `sn-inspect player --body`):** the body is the
  `Player` object's own hierarchy (134 nodes; 27 renderers, 25 of them
  skinned, the suits on layer 8, which the main camera's mask includes).
  `Player.equipmentModels` has 3 slots: `Body` (a default model and 4
  models for 3 suits: the radiation suit has two), `Gloves` (a default
  and 2) and `Foots` (no default, 3 fin models). Every model reference
  finds a node.
- **Confirmed (code):** `Player.Start` calls `EquipmentChanged` with
  nothing equipped, so a new game shows each slot's default model and
  hides every other model (the stored scene has several suits active).
  The rule: a model is active when its tech type equals the slot's
  (`Equipment.GetTechTypeInSlot`, `TechType.None` = 0 when empty); the
  default is active when no model matched. `StartHideGlovesFor` (the
  infection reveal) hides the gloves for 12 s: not ported (story).
- **Confirmed (real data):** a new game draws 3 renderers: the dive
  suit's body, hands and head, all `MarmosetUBER` (4 sub-meshes), all
  skinned. The head (`Player.head`) is stored with `m_CastShadows`
  "shadows only"; `SetHeadVisible(true)` (Cyclops and scanner room
  cameras only) makes it drawn. Hidden renderers use other shaders too
  (`Legacy Shaders/Transparent/Diffuse` on the radiation helmet glass,
  `UWE/SIG Alpha Border`, `UWE/SIG` and `Legacy Shaders/Diffuse` on the
  inactive scuba parts and `player_head`); they don't show in a new game.
- **Confirmed (real data):** `MainCameraControl.viewModel` is `body`, at
  the player's origin with no rotation, as are `camRoot` (the
  `MainCameraControl`) and `player_view` (the animator, with the
  `ArmsController`). `cameraUPTransform` is `camOffset`, 0.063 m above and
  0.15 m behind `camRoot` in the player's axes: looking up turns about
  that point. How the drawn camera (`PlayerCameras/MainCamera`, a
  separate top-level object) follows the player was not found in the
  code yet (searched: `SNCameraRoot`, `MainCameraControl`, `Player`,
  `CameraToPlayerManager`): M9g2.
- **Confirmed (real data):** `ArmsController`: `smoothSpeedUnderWater`
  10, `smoothSpeedAboveWater` 15 (the code's initial values are 4 and 8;
  the scene's count), `turnAnimationDampTime` 0 (so `view_turn` is set
  undamped), `ikToggleTime` 0 (IK weights jump to 0 or 1). The dive scan
  interval (0.5 s) is a private field: code only.
- **Confirmed (code), the animator's parameters:** of the controller's
  201, the rules M9g ports set 17 every frame with values that change in
  play (`ArmsController.Update`, `SetPlayerSpeedParameters`,
  `UpdateDiving`, the `on_surface` rule of `InstallAnimationRules`;
  `Player.Start`: `vr_active`; `Player.OnKill`: the three death triggers
  by damage type). The same scripts set 12 more every frame to false
  while there is no tool, PDA, builder, vehicle, piloting, Bleeder or
  grab/bash by a creature. The other 172 stay at their defaults until
  their item: 48 `holding_*` (`PlayerTool.animToolName`: the held
  tool's tech type, lower case), the cinematics (set by name from the
  data: `PlayerCinematicController.animParam` and
  `playerViewAnimationName`), the PDA, vehicles, bases, creatures and the
  story. Two death triggers (`player_death_choke`, `player_death_drown`)
  and a few others (`swimming_fast`, `is_punching`, `chestAim_*`) are
  named by no string in the decompiled code; they may be set from data
  or not at all (**not checked**). `using_mechsuit`, which
  `InstallAnimationRules` sets, is not a parameter of this controller
  (`SafeAnimator` ignores it). The lists are `RULE_PARAMETERS` and
  `FIXED_PARAMETERS` in `player_body.rs`; the test checks each is in the
  controller.

## The Aurora's explosion (M7f4e)

- **Confirmed (code):** `CrashedShipExploder.SetExplodeTime` sets
  `timeToStartWarning` = the clock now (`DayNightCycle.timePassedAsFloat`)
  and `timeToStartCountdown` = warning + `Random.Range(2.3f, 4f)` ×
  `1200f`. It runs on the exploder's first `Update` once
  `LargeWorldStreamer.IsReady()`, only if the exploder was not loaded
  from a save (or the save is older than version 2). The numbers are read
  from the DLL: the `Random.Range` bounds and the multiplier from that
  method's IL (`sn_dotnet::random_range_f32`, `multiplier_f32`), the
  delays from the `const` fields `delayBeforeExplosionSound` 24,
  `delayBeforeExplosionFX` 25, `delayBeforeSwap` 27.
- **Confirmed (code):** each `Update` feeds the clock to a
  `ScalarMonitor` and checks, in one `else if` chain, countdown + 27
  (`SwapModels(true)`), + 25 (explosion effects, camera shake, explosive
  force, `RadiusDamage(2000, …, 500)`), + 24 (explosion sound), + 0
  (countdown sound, `OnShipExplode`, warning effects). "Just went above"
  is `previous ≤ t < current`, so a frame that jumps past several
  thresholds fires only the latest. `IsExploded` = clock > countdown + 27.
  `Start` shows the intact ship unless loaded from a save.
- **Confirmed (code):** a new game's clock starts at `timePassedOrigin` =
  1200 × (9:36:00 of `dateOrigin`) / 86400 = 480 s, so the countdown comes
  2,760–4,800 game seconds (46–80 real minutes) after the start.
  `AuroraWarnings` triggers story goals at 20, 50 and 80 % between warning
  and countdown (not ported: PDA messages).
- **Confirmed (parser, real data):** `disableOnExplosion` is
  `starship_unexploded` and `unexplodedFX`, `enableOnExplosion` is
  `starship_expoded` and `explodedFX`, `explodedExterior` is
  `starship_expoded` (the whole wreck model). Intact: 330 nodes drawn,
  exploded: 337; split by state: 11 drawn only intact, 18 only exploded
  (all of them in the exterior), 319 in both.
- **Confirmed (code):** `ShipExteriorCullManager.Update` runs every
  `updateEveryXFrames` frames (`Time.frameCount % n == 0`; **10** in the
  Aurora scene, read) and calls `CullExplodedExterior(!inside)`, where
  inside = the camera's position is strictly inside one box of any
  registered `ShipExteriorCull` (`PointInOABB`: the point in the box
  collider's Transform space minus its center, against ± size / 2). A
  `ShipExteriorCull` registers in `Start` and leaves in `OnDestroy`.
- **Confirmed (parser, real data):** only two prefabs have a
  `ShipExteriorCull` (of 3,336 world prefabs): `CrashedShip_cargo_room`
  (4 boxes) and `CrashedShip_exo_room` (3 boxes), each placed once; all
  scripts enabled, every listed collider a `BoxCollider` of its prefab.
- **Confirmed (code):** `CullExplodedExterior` acts only when the exploder
  is both `initialized` and `deserialized`. A new game sets only
  `initialized`; a loaded save sets `deserialized` and skips the
  initialisation. So in a game started in this session the exterior is
  **never** hidden. **Hypothesis:** in a loaded save too, unless the
  exploder's first `Update` runs before its data is restored (or the save
  is a legacy one): the load order (`LargeWorld` sets the streamer ready
  before `LoadSceneObjectsAsync`) was read but not traced to the
  exploder's own scene load. Revisit with saves (M9f).
- **Confirmed (code, real data):** `DisableBeforeExplosion` (active only
  if the ship has exploded when it starts) is on one prefab,
  `ExplodedWreckage`, which is not placed in the world.
- **Our stand-in (not the game's):** the countdown's `Random.Range` draw
  comes from our seeded generator (`--lifepod-seed`; seed 1: 4,500 s);
  the game's `Random` is unseeded, so any value in the range is the
  game's behaviour.

## Shadow casting modes (M7f4e)

- **Confirmed (parser, real data):** drawn renderers of the placed
  prefabs by `m_CastShadows`: off 5,380 (29,145 placements), on 6,776
  (202,091), two-sided 0, shadows only 1 (1 placement). The Aurora scene:
  off 195, on 153. The client now leaves "off" renderers out of the sun's
  shadow map; the shadows-only one is drawn as a normal renderer (not
  1:1).

## The lifepod's light (M7f4g)

- **Confirmed (code):** `MarmoLifepodSky` (on the pod) listens to
  `Player.escapePod`: true → `SkyManager.GlobalSky = anchorSky`, false →
  `MarmoSkies.GetSky(SafeShallow)`. The escapepod scene has **no**
  `SkyApplier`, so the pod's hull and interior are lit by the global
  sky, as is every renderer in the world without an applier: while the
  player is inside, they all take the pod's sky.
- **Confirmed (code):** a `SkyApplier` with anchor Auto, BaseInterior or
  BaseGlass and a `MarmoLifepodSky` among its parents takes that
  `anchorSky` (`GetEnvironment`, `GetSkyForEnvironment`). The modules the
  pod spawns are its children (`PrefabSpawnBase` spawns under its own
  Transform unless `attachToParent` is set), and `EscapePod.Start`
  re-sends the environment to them after 0.5 s (`ForceSkyApplier`).
- **Confirmed (code):** `Player.escapePod` is set by `EscapePod.Awake` in
  a new game and by `RespawnPlayer`, set and cleared by the hatches
  (`EnterExitHelper`, `isForEscapePod`), and cleared when the player is
  more than `escapePodRadius` (15 m) from the pod
  (`ValidateEscapePod`).
- **Confirmed (code):** `LightingController`: `state`, `fadeDuration`,
  `skies[]` (`MultiStatesSky`: an `mset.Sky` and its master, diffuse and
  specular intensity per state), `lights[]` (`MultiStatesLight`: a `Light`
  and its intensity per state; its GameObject is switched on when the
  intensity is above 0 and off at exactly 0), `emissiveController`
  (`MultiStatesEmissive`: an intensity per state, `_UwePowerLoss =
  clamp01(1 − i)` on the renderers of appliers with `emissiveFromPower`
  below it). `SnapToState` sets the values; `LerpToState(s, t)` fades
  linearly from the current values over `t` seconds, then snaps.
- **Confirmed (code), a quirk:** `Update` compares `state` with
  `prevState`, which only `Update` sets; when they differ it calls
  `LerpToState(state)` with `fadeDuration`. So a `LerpToState(s, 5)` from
  another script (`EscapePod`, `IntroLifepodDirector`) is restarted on the
  next frame as a `fadeDuration` fade (1 s for the pod) from the values
  reached. Our port does the same (unit test).
- **Confirmed (parser, real data, bytes checked with UnityPy):** the
  pod's controller stores state 0 (Operational) and a fade of 1 s; one
  sky, the `anchorSky` (`SkyEscapePod`, also the cinematic's
  `interiorSky`); per state Operational / Danger / Damaged: master 10 /
  0.8 / 2.5, diffuse 2 / 0.5 / 0.8, specular 1.5 / 3 / 1; three lights
  (`SoptLight_Red` ×2: spot, range 8 m, 120°, intensity 0 / 1.25 / 0;
  `PointLight_RedAmbient`: point, range 6 m, 0 / 0.22 / 0), their
  GameObjects inactive as stored; emissive 0 / 1 / 1. The sky stores the
  Operational values (master 10, diffuse 2, specular 1.5), is not
  affected by the day (`_AffectedByDayNightCycle` 0, so it adds its own
  ambient), and is turned 90° about y.
- **Confirmed (code):** a new game with the intro: `OnIntroStart` snaps
  to 0; the explosion drives the sky's master intensity by
  `skyIntensityCurve` and runs the lights animator
  (`Life_Pod_lights_controller`, bool `aurora_exploding`); `OnDamagedPod`
  snaps to 1 (red alert) and disables the animator;
  `ConcludeIntroSequence` (the fire is out) lerps to 2. Skipping the
  intro (`StopIntroCinematic(interrupted)`) runs `StopAll` (snap to 1,
  animator off, `HatchLight` off) and then snaps to 2. Both end in state
  2 (Damaged) until the pod is repaired (`UpdateDamagedEffects`: health
  above 99 % → `LerpToState(0, 5)`). Without the intro (a mode that
  spawns starting items) nothing changes the stored state 0.
- **Confirmed (code):** the lights animator's idle state has no motion
  (and "no write defaults"), so disabling it changes no values.
- **Ours:** the client starts in state 2 (`--lifepod-state` to choose),
  the pod's sky set by the controller every frame, the global sky's
  materials relit when `in_pod` changes, the three lights driven by the
  controller. With the fly camera (no player): in the pod at its spawn,
  out beyond 15 m (the game's rule for leaving without the hatch).
  **Not ported:** `_UwePowerLoss` (our object shader has no such input;
  equal in the default state, where the emissive intensity is 1); the
  pod's `AtmosphereVolume` (M8c7b); the intro's sky curve and animator
  (with the intro).

## Pick-ups and outcrops

- **Confirmed (census):** `Pickupable` is on 50 placed prefabs (23,832
  placements) and on 106 more prefabs that only the spawn slots place.
- **Confirmed (parser):** `BreakableResource` (`prefabList` of
  `AssetReference` + tech type + chance, a default prefab and tech type,
  `verticalSpawnOffset`, `numChances`, `hitsToBreak`, four PPtrs,
  `breakText`, `customGoalText`) is on 4 prefabs (1 placed, 3 from the
  slots); all parse to their last byte; `hitsToBreak` is 3 on all.
  An `AssetReference` is three strings (GUID, sub-object name, sub-object
  type).
- **Hypothesis:** the tech type of a pick-up comes from its prefab
  (`EntTechData` or the world entity info), not from `Pickupable`, which
  has no tech type field. Checked in M9d.

## Fabricator menus: `CraftTree` (code only)

- **Confirmed (code):** each crafting machine's menu (fabricator,
  constructor, workbench, vehicle upgrade console, rocket, …) is built in
  C# by nested `new CraftNode(id, TreeAction, TechType)` calls joined by
  `AddNode`. There is no data file for it.
- **Done (P1):** read from the IL of those methods in the player's
  `Assembly-CSharp.dll` at runtime: 7 menus, 159 nodes, 134 craft nodes,
  each with a TechData entry. Details in `dotnet.md`. There is no rocket
  menu: `RocketScheme` exists but is never used.

## Tech type names: the `TechType` enum (code only)

- **Confirmed (code):** `TechType` is a C# enum. The JSON and the
  serialized scripts use its numbers; names are needed to match the
  language files and the code's own references.
- **Confirmed (P1, metadata):** 793 members with 793 distinct values, read
  from the enum's fields and constants (`dotnet.md`). An earlier count of
  787 here, from the decompiled source, was a miscount: the source has 793
  too.

## Text

- **Confirmed (file listing):** `StreamingAssets/SNUnmanagedData/LanguageFiles/`
  holds one JSON file per language. Not opened yet; key format not checked.
