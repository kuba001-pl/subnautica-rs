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
  `cameraTiltMod` 0.1, `camPDAZOffset` 0.18. **Corrected in M9g2:** the
  main camera is not at `camRoot` but 0.063 m up and 0.15 m back from it
  (§ The player's body).
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
- **Confirmed (code), M9g3:** `GroundMotor.IsGrounded()` is the walking
  motor's own flag (`PlayerMotor.grounded`, initially true), changed only
  in `GroundMotor`'s update (falling off, landing, jumping). It keeps its
  last value while the player swims, is dead or is moved by a hatch or a
  respawn. `sn-sim`'s `Player::walk_grounded` is that flag; the body's
  falling clock and speeds read it. With our `grounded` (cleared by
  swimming and teleports) the respawned player, held still in the pod
  for 2 s, counted as falling and played the falling animation.
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
  that point. Where the drawn camera hangs: below (M9g2).
- **Confirmed (real data, client M9g4):** the player's `Animator` has
  culling mode 1 (cull update transforms); its rig is 113 nodes with 213
  animated slots, and the three drawn skins use 43, 36 and 5 bones, all
  inside the rig. Standing in the pod, the animated nodes reach 1.53 m
  below the view model, matching the standing capsule's bottom (1.50 m
  below the player's transform).
- **Confirmed (real data, `sn-inspect scene escapepod --cinematics`,
  real-data test `player_movement_data`; M9g5a):** the pod's 8 hatch
  triggers and the intro each have a `PlayerCinematicController`; all
  move the player along `models/Life_Pod_damaged_03/root/player_cineLoc/
  cin_target`, a node of the pod's own animator (`escape_pod_controller`
  on `Life_Pod_damaged_03`); the hatches interpolate 0.25 s in and out,
  none enforces the end by time. Each sets a pod parameter
  (`escapepod_topout`, `_topin`, `_botin`, `_botout`, `_botout_first`,
  `_topout_first`, `_left_side`, `_right_side`), a "prepare" parameter
  during the move in (`prepare_…`), and the player's parameter of the
  same name (both side hatches: `escapepod_side`). Boarding calls
  `EnterExitHelper.CinematicEnter` at the start of the cinematic,
  leaving calls `CinematicExit` at its end.
- **Confirmed (real data):** the end of a hatch cinematic is the
  `OnPlayerCinematicModeEnd` event at the last frame of the pod's clip
  (`top_out` 2.000 s, `top_in` 1.167 s, `bot_in` 1.667 s, `bot_out`
  0.667 s, `side` and `side2` 2.667 s, `escapepod_first_topout` 8.167 s,
  `escapepod_first_botout` 6.333 s). The event arrives on
  `Life_Pod_damaged_03`, whose one `OnPlayerCinematicModeEndForward`
  passes it to all 8 hatch controllers; only the running one acts. The
  first-use clips carry 1–3 more events (creatures leaving the pod's
  surroundings: not used without creatures).
- **Confirmed (real data):** `Player.camAnchor` is the skeleton's `Cam`
  bone; `CameraToPlayerManager.headCameraBone` is `cam_deathpos`.
- **Confirmed (code, `PlayerCinematicController`):** a cinematic starts
  only when none runs; the player's transform lerps (position) and
  slerps (rotation) to the animated node over the interpolation time
  while `camRoot` moves from its offset to `camAnchor`; then the
  parameters are set and the player sits on the node every frame (late
  update) until the end event; the end puts the player on the node,
  clears the parameters, and then lerps to `endTransform` only if it is
  set and not VR-only, else stops there. `MainCameraControl.cinematicMode`
  folds the look into the player's rotation at the start and takes it
  back at the end (pitch clamped to the look limits; the rest of the
  pitch and the roll stay on the player's transform and
  `Player.UpdateRotation` eases them to 0 at 10 per second).
- **Measured (M9g5d, `sn-inspect walk`/`dive`, seeds 1–5):** at our 50 Hz
  step the bottom hatches take 6.62 s (first-use exit), 0.96 s (exit)
  and 2.22 s (entry): the 0.25 s moves take 13 steps and the pod's
  animator starts its clip one step after the parameter is set. The
  first-use exit ends where the animation leaves `cin_target`, 0.48 m
  from the stored (VR-only) end point; the normal exit's animation ends
  on its stored end point (0.00 m); the entry ends on `botin_end`. The
  oxygen doesn't change during a cinematic.
- **Hypothesis (M9g5d):** where the first-use exit leaves the player,
  the swimming capsule overlaps the pod by 0.27 m (every seed). PhysX
  separates an overlapping rigid body; the player's `Rigidbody` sets no
  `maxDepenetrationVelocity` (Unity 2019.4 serializes none, the game's
  code sets none), so we take PhysX's unbounded default: out in one
  step, along the overlap's normal, with no velocity added. Not checked
  against the game.
- **Hypothesis (M9g5e fix):** the walking motor does the same. The top
  hatch's normal exit (`top_out_trigger`, end point VR-only) leaves the
  player where the animation ends, 0.30 m inside the pod's roof; the
  game's `CharacterController` recovers from overlaps
  (`enableOverlapRecovery`, on by default in Unity's documentation; the
  game's code never sets it). Ours pushes out in one step (0.45 m up,
  onto the roof at the height the first-use exit leaves the player).
  How PhysX's overlap recovery moves the character exactly: not checked.
- **Confirmed (code, `PlayerCinematicController.OnPlayerCinematicModeEnd`):**
  the end event does nothing when no cinematic runs
  (`!cinematicModeActive`). The first-use clips fire it on two layers in
  one frame; when the first call ends the cinematic (no end point used),
  the second must leave the player alone. Ours put the player back on
  the animated node, so the node's yaw stayed on the player's transform
  while the look had taken it back: the view turned twice and movement
  no longer went where the camera looked (the bottom first-use exit,
  270°). Fixed (M9g5e); `sn-inspect walk`/`dive` now check that no yaw
  is left on the player after each hatch.
- **Confirmed (code, `Player.OnKill`, `CameraToPlayerManager`):** death
  sets `player_death` (fire: `player_death_fire`, explosions:
  `player_death_explosion`) and stops `MainCameraControl`; `camRoot`
  then copies `cam_deathpos` every frame until the respawn. The death
  clips carry `DeathFade` / `DeathCut` events (the screen's fade).
- **Confirmed (real data, M9g2): where the camera is.** The scene's one
  `AutoParent` sits on the top-level `PlayerCameras` (which holds the
  camera tagged `MainCamera`, at identity in its object) and on `Start`
  puts it under `cameraOffsetTransform` (`pdaCamPivot`, below `camOffset`)
  with identity locals (`makeLocalsIdentity` on). So the eye is
  `camRoot` (at the player's origin) → `camOffset` (0, 0.063, −0.15) →
  `pdaCamPivot` (a few micrometres from `camOffset`, editor rounding).
  Looking down turns `camRoot`, so the eye swings about the player's
  origin; looking up turns `camOffset`, so the eye stays put.
  `sn-inspect player --body` prints the eye at rest: 0, 0.0628, −0.15 m.
  This corrects M9c's "the camera sits at the player's transform".
- **The rules ported in `sn-sim::body` (M9g2; code, read):**
  - Speeds (`SetPlayerSpeedParameters`): the velocity in the camera's
    frame (`InverseTransformDirection` of the camera's rotation) under
    water or when not grounded; else its dot products with the camera's
    forward and right flattened to the ground (so pitch doesn't matter
    when walking). Smoothed with `Vector3.Slerp(smoothed, new, speed ×
    dt)`. `Vector3.Slerp` is Unity's native code: our version turns the
    direction by the fraction of the angle and blends the length
    linearly, a linear blend when either vector is zero (**hypothesis**).
  - `view_pitch` (`GetCameraPitch`): the x angles of `camRoot`,
    `cameraUPTransform` and `cameraOffsetTransform` summed, wrapped to
    ±180°, negated: equal to `rotationY` without camera shake.
    `view_turn`: `Mathf.DeltaAngle` of the view model's world yaw since
    the last frame ÷ dt (the first frame compares with 0, as the game's
    field starts at 0); damp time 0, so set as computed (**hypothesis**:
    Unity's damped `SetFloat` with damp time 0 sets the value).
  - The falling animation (`jump`, `Player.GetPlayFallingAnimation`):
    falling (`FixedUpdate`: not under water, not grounded, no cinematic)
    for 0.45 s; `OnJump` sets it at once (falling began 0.45 s ago).
  - Diving (`UpdateDiving`): out of water, not grounded, falling
    (velocity y < 0); every 0.5 s a ray from the player along the
    velocity's direction + down, normalised, 5 m long or, above y = 0,
    to 1.5 m under the surface along it; a hit gives `diving_land`, none
    `diving`.
  - `on_surface`: within 1 m of the water level, not inside (the pod),
    swimming.
  - Camera (`OnUpdate`, normal play): `swimCameraAnimation` ±1/s with
    `UnderWaterTracker` (= `Player.IsUnderwater`, each physics step); the
    swim bob `(sin(6 t) − 1) × (0.02 + 0.15 × s) × swimCameraAnimation`,
    `s` moving towards min(1, speed / 5) by dt (`UWE.Utils.Slerp` for
    floats is a move-towards: read in `Assembly-CSharp-firstpass`),
    with the camera-bobbing option (on by default); the landing bob
    (`OnLand`: force = clamp(−impact y, 0, 15); bob += force × dt up to
    0.9, force −= max(1, force) × 5 dt, bob −= √bob × 3 dt); strafe tilt
    under water (−12°/s × strafe input, clamped ±10°, then 4°/s back to
    0, so it settles at ∓9.92°). The view model takes `camRoot`'s yaw and
    its local position (the bobs).
  - Death triggers: every death we have (suffocation, falls) is
    `player_death` (fire and explosion pick the other two).
  - **Hypothesis:** in one frame `ArmsController.Update` runs before
    `MainCameraControl.OnUpdate` (a `ManagedUpdate` queue), so the arms
    read the camera as the last frame left it.
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
  shadow map. Since M9g4 "shadows only" renderers (this one and the
  player's head) are put on a render layer only the sun sees, so they
  cast its shadow and are not drawn (**confirmed** in the client by
  counting: off the camera's layer, in the sun's shadow pass). Local
  lights cast no shadows in the client yet.

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

Readers (M9d1): `sn-unity::items` (layouts), `sn-assets::items`
(`item_prefab`, `TechLookup`, `addressable_prefab`, `player_items`,
`language`). Tool: `sn-inspect pickups`.

- **Confirmed (census):** `Pickupable` is on 50 placed prefabs (23,832
  placements) and on 106 more prefabs that only the spawn slots place
  (P0; the same numbers again in M9d1). No prefab has more than one.
- **Confirmed (code, M9d1 layout check):** `Pickupable`'s serialized
  fields are five bools: `isPickupable`, `destroyOnDeath`,
  `randomizeRotationWhenDropped`, `activateRigidbodyWhenDropped`,
  `usePackUpIcon` (its base `HandTarget` has none; the protobuf fields are
  `[NonSerialized]`). All 156 parse to their last byte. Counts: 59 have
  `isPickupable` false (the hand cannot take them: `AllowedToPickUp`),
  70 `destroyOnDeath` false, 54 `randomizeRotationWhenDropped` false, 0
  `usePackUpIcon`; 20 have an `Eatable`.
- **Confirmed (code):** an object's tech type is `CraftData.GetTechType`:
  from the object up its parents, the first `TechTag` (`type`) or
  `PrefabIdentifier`; for the identifier, its class id → the prefab
  database's file name without extension → `EntTechData` by that name,
  case-insensitive (`OrdinalIgnoreCase`); unknown → `None`. This settles
  P0's hypothesis: `Pickupable` has no tech type of its own.
- **Confirmed (layouts, real data):** `TechTag` is one `i32`;
  `PrefabIdentifier` holds only the class id string (72 bytes for a
  36-character id; an earlier guess of two strings failed on all 1,543
  prefabs, the object's own id is not in the asset).
- **Confirmed (real data, M9d1):** of the 156 pick-up prefabs, 72 get
  their tech type from a `TechTag`, 84 from the class id. One has `None`:
  a prop inside `DeepGrandReefAbandonedBase` (`discovery_lab_props_02`,
  `isPickupable` true, 1 placement), whose nearest identifier is the
  base's own. **Hypothesis:** the game gives `None` too (by the rule
  above; not checked in the running game). 101 distinct tech types; all
  101 have a name in `English.json`; 70 have a TechData entry, 15 give an
  `itemSize` (all 2×2), the other 86 are the default 1×1. Equipment types
  among them (value: tech types): 0 (none) 66, 1 31, 8, 9, 11, 13 one
  each.
- **Confirmed (parser):** `BreakableResource` (`prefabList` of
  `AssetReference` + tech type + chance, a default prefab and tech type,
  `verticalSpawnOffset`, `numChances`, `hitsToBreak`, four PPtrs,
  `breakText`, `customGoalText`) is on 4 prefabs (1 placed, 3 from the
  slots); all parse to their last byte; `hitsToBreak` is 3 on all.
  An `AssetReference` is three strings (GUID, sub-object name, sub-object
  type).
- **Confirmed (real data, M9d1):** each `AssetReference` GUID is a key of
  the Addressables catalog naming a GameObject; all 9 resources of the 4
  outcrops resolve, and each prefab's tech type by the rule above equals
  the tech type listed next to it. All 4 have `numChances` 1 and
  `verticalSpawnOffset` 0.2; limestone has 1 chance entry, sandstone and
  shale 2, the placed coral piece only a default; their `breakText` keys
  are all in `English.json`. The placed coral piece's own tech type is
  `None` (it only needs its `breakText`).
- **Confirmed (code):** the draws use `PlayerEntropy.CheckChance`, which
  goes to the `FairRandomizer` of the entry's tech type (none → false).
  `FairRandomizer` serializes `timeLastCheck` and `entropy` (`version` is
  `[NonSerialized]`).
- **Confirmed (real data):** the main scene's `PlayerEntropy` has 8
  randomizers, each pointing to its own `FairRandomizer`, all with
  `entropy` 0 and `timeLastCheck` −1. Every chance entry of the 4 outcrops
  names one of these 8 tech types.

## Inventory (M9d)

- **Confirmed (code, IL read in M9d1):** the player's container is
  `new ItemsContainer(6, 8, …)` in `Inventory.Awake`: 6 wide, 8 high.
- **Confirmed (code, IL):** `Equipment.slotMapping` has 38 slots (slot
  name → `EquipmentType`); the player's are the 7 of
  `Inventory.UnlockDefaultEquipmentSlots` (head, body, gloves, feet, two
  chips, tank), all in the table. A slot takes an item whose
  `EquipmentType` equals its own (vehicle modules also fit Seamoth and
  Prawn suit slots: `Equipment.IsCompatible`).
- **Confirmed (code), ported in M9d2 (`sn-sim::inventory`), not yet
  checked against the game's behaviour:**
  - **Packing** (`ItemsContainer.TrySort`): items are grouped by tech
    type (each group's size is TechData's `itemSize`, default 1 × 1).
    Groups are ordered by `GroupComparer`: larger long side first, then
    larger area, at equal area the taller first, then the lower tech type
    number. Each item takes the first box that is free (`GetRoomFor`:
    rows top to bottom, in each row left to right, jumping past the item
    found in the way, `NextFreeCell`). Every check and every change
    re-packs the whole grid, so a container can take an item that would
    not fit in the cells left free by the current layout.
  - **`HasRoomFor`** packs a trial with a "ghost" group of the new size
    (id 0, `TechType.None`); a swap leaves one item out of the trial
    (`ignoreForSorting`).
  - **Enumeration order** (what `LoseItems` walks): the groups in the
    `Dictionary<TechType, ItemGroup>`'s order, each group's items in the
    order added, then the equipment's slots in the order they were added.
    A .NET dictionary reuses a removed entry's slot for the next insert,
    last freed first (from the runtime's source; **not tested** in the
    game).
  - **`RemoveItem(TechType)`** takes the group's last item, except that
    among the batteries at the end of the group the one with the least
    charge goes.
  - **Equipment:** `GetFreeSlot` walks the slots type by type (types in
    the order their first slot was added) and takes the first empty
    compatible one; `GetCompatibleSlotDefault` does the same but, if none
    is empty, gives the first slot of the first compatible type (a swap).
    An item of `EquipmentType.None` fits no player slot.
  - **`Inventory.Pickup`:** refuses unless the **grid** has room, even
    for an item that would go to a free slot. Then, unless the object has
    an `Eatable`, the item goes into a free slot of its type, else into
    the grid; then `KnownTech.Analyze`. Picked up inside a sub or base
    (`Utils.GetSubRoot()`), `destroyOnDeath` is cleared.
  - **The 1 s after a drop does not apply:** `AllowedToPickUp` asks
    `Time.time − timeDropped > 1`, but `Pickupable.Drop` sets
    `timeDropped = 0` (the field is never set to a time). So a dropped
    item can be picked up at once (from 1 s after the game started).
    This corrects the M9d plan.
  - **`KnownTech.Analyze(tech)`:** only the first `analysisTech` entry for
    the type counts (the type is then in `analyzedTech`); each unlocked
    type is `Add`ed, and `Add` analyses it in turn (chains). The default
    tech is added the same way at start (`AddRange`). Compound tech,
    scanner entries and story goals are not ported.
  - **`AddOrSwap`:** moving onto a taken target removes it first, moves
    the dragged item, then puts the removed item where the dragged one
    was; if that fails, both go back. Moving within the same grid is
    refused; within the same equipment only to another slot.
  - **Drop:** `CanDropItemHere` refuses in a sub or base (unless in a
    water park), in the lifepod, while piloting, and out of the water in
    a Precursor base. `Inventory.RayCast(camera, forward, 10, 0.75, 1.5)`
    turns the camera's forward by `Quaternion.Euler(10 cx, 10 cy, 0)`
    (`cx, cy` = `Random.insideUnitCircle`, about the **world's** axes),
    casts `Random.Range(0.75, 1.5) + 0.2` m (layers: all but Player and
    TouchScreen) and stops 0.2 m before a hit. `FindDropPosition` then
    casts a 0.3 m sphere from the camera **5 m** towards that point
    (not just to it; triggers ignored, the player's colliders skipped,
    contacts at distance 0 skipped) and, at the nearest contact, puts the
    item 0.15 m back from the contact point along the cast.
  - **`LoseItems`:** collects every item with `destroyOnDeath` (grid,
    then slots) and drops all but the last (`i < Count − 1`).
  - **Outcrops:** a bare-hand click is a hit (`hitsToBreak--`; it breaks
    only when that reaches exactly 0, so an outcrop with 0 never breaks by
    hand); with a bashing tool the bash breaks it at once, so does a
    Prawn suit. A broken outcrop never breaks again. Each of `numChances`
    draws takes the first `prefabList` entry whose
    `PlayerEntropy.CheckChance(tech, chance)` passes; if no draw found
    one, `defaultPrefab` once. `PlayerEntropy.CheckChance` is **false
    for a tech type without a randomizer, even at chance 1**; with
    several for one type all are tried and the last answers.
    `FairRandomizer.CheckChance`: true at ≥ 1, false at ≤ 0 (no draw);
    else `entropy += chance; if entropy > Random.value { entropy −= 1;
    true }`. The resource appears at `position + up × verticalSpawnOffset`
    with a torque about the **world x axis** of `Random.Range(3, 6)`
    (the int overload: 3, 4 or 5) and a force of `up × 0.1`.
  - **The lifepod's supplies:** each item goes in if `HasRoomFor`, else
    it is destroyed.
  - **`grab`:** `timeGrabbed != 0 && timeGrabbed + 0.4 > Time.time`.
- **Confirmed (real data):** the lifepod's locker is the escape pod
  scene's one `StorageContainer`, the one `SpawnEscapePodSupplies`
  points to: 4 wide, 8 high, its hover and label keys in `English.json`.
  A new game fills it with `LootSpawner.escapePodTechTypes`: 6 items
  (two each of three tech types). `LootSpawner` is on a prefab that the
  `essentials` scene's `PrefabListSpawner` creates (one of 21); its
  `supplyTechTypes` (supply crates, later) has 6 entries.
- **Confirmed (real data):** `PDAData.analysisTech` has 126 entries (86
  unlocks); 19 of them are for a pick-up tech type, so picking those up
  unlocks blueprints.
- **Confirmed (code and real data, M9d4a):** the lifepod's locker is
  opened by the escape pod scene's one `GenericHandTarget`, on the node
  `StorageHandTarget` (a child of `EscapePod`). Its hover and click events
  call `EscapePod.StorageHover` (hand text `EscapePodStorageOpen`) and
  `EscapePod.StorageOpen` (the animator bool `storage_open`). The
  `StorageContainer` node itself has no collider; the hand target's node
  has 1.
- **Confirmed (code):** the prefab spawned for a tech type
  (`CraftData.GetPrefabForTechTypeAsync`, the lifepod's supplies) is
  `techMapping[tech]`. `PreparePrefabIDCache` builds it by walking
  `PrefabDatabase.prefabFiles` (`prefabs.db`, file order) and giving each
  class id the tech type of its file name (`EntTechData`). A tech type
  named by several prefabs keeps the **last** one.
- **Confirmed (real data, M9d3d):** `sn_assets::item_rules` over the 1,543
  candidate prefabs (placed or in the loot table; 0 unreadable). It also
  reads 2 more, named only by the supplies: 158 pickupable class ids of 102
  tech types (M9d1's 101, plus one only the supplies name). 4 outcrop
  classes; 124 TechData entries with a size, 124 with an equipment type;
  the tech type → prefab map has 515 entries. Every outcrop resource and
  every supply is a pickup of its listed tech type, and the 6 supplies fit
  the 4 × 8 locker.

## The hand on items (M9d5)

- **Confirmed (code):** `Pickupable.OnHandHover`: when `AllowedToPickUp`
  (pickupable and room), the hand text is `PickUpFormat` formatted with
  the item's name (`LanguageCache.GetPickupText`; `PackUpFormat` with
  `usePackUpIcon`, on no prefab); pickupable without room: the name with
  the subscript `InventoryFull`; not pickupable: the name only. A click
  that picks up calls `Player.PlayGrab`. `BreakableResource.OnHandHover`
  shows its `breakText`; a click without a bash tool plays the grab and
  hits. The lifepod's locker (`GenericHandTarget` → `EscapePod`) shows
  `EscapePodStorageOpen`; `StorageOpen` sets the pod animator's bool
  `storage_open` and opens the storage 0.25 s later. These keys are
  literals in the code.
- **Confirmed (code):** `HandTarget.isValidHandTarget` (false for 2 s
  after an object is created) is not asked by `GUIHand` before
  `OnHandHover` / `OnHandClick`; only some targets (beds, benches,
  chairs, consoles, cinematic triggers) check it themselves.
  `Pickupable` and `BreakableResource` do not, so a fresh item can be
  taken at once.
- **Confirmed (real data, M9d5a):** the locker's hand collider is at
  floor height on the side of the pod across from the player's spawn
  (3.4 m from the spawn's camera, beyond the hand's 2 m), centred
  0.78 m above the pod's origin; its node holds one collider. Reached by
  walking round the ladder: the hand's ray hits it from 1.98 m.

## The inventory panel and the locker (M9d5b)

Read with `sn-inspect code --il <TYPE> <METHOD>` (`METHOD#N` for an
overload) and `code --enum <TYPE>` (`Outer/Inner` for a nested enum).

- **Confirmed (code):** `ItemAction` is a flag set: None 0, Use 1, Eat 2,
  Equip 4, Unequip 8, Assign 16, Switch 32, Swap 64, Drop 128.
  `GameInput.Device`: Keyboard 0, Controller 1. `GameInput.Button` 1 is
  `PDA`.
- **Confirmed (code):** `uGUI_ItemIcon.OnPointerClick` passes the pointer
  button as 0 left, 1 right, 2 middle, through `uGUI_ItemsContainer` to
  `uGUI_InventoryTab.OnPointerClick`, which (unless an item is being
  dragged) runs `Inventory.ExecuteItemAction(item, button)` if
  `GetItemAction(item, button)` is not None.
- **Confirmed (code):** `GetItemAction` with the keyboard: button 0 gives
  the first of Use, Eat, Equip, Unequip, Swap, Switch that
  `GetAllItemActions` allows; button 1 Drop if allowed; anything else
  None. With a controller, buttons 2 and 3 give Drop and Assign.
- **Confirmed (code):** `GetAllItemActions(item)`: the opposite store is
  `GetOppositeContainer` (from the player's grid: the first used
  storage with `HasRoomFor`, else the first used storage, else the
  equipment; from anything else: the grid). For an item of the player's
  grid: when the opposite is the equipment, Eat if the item has a
  component (a generic `GetComponent`, **hypothesis:** `Eatable`) and the
  game mode is not `NoSurvival` (or it is `Bladderfish`, 2515, without
  `NoOxygen`), Use for `FirstAidKit` (4514); Drop if
  `CanDropItemHere`; Assign if bindable. Then: from the equipment to the
  grid Unequip if `CanDrag` and the grid `HasRoomFor`; into the equipment
  Equip if `GetFreeSlot` of its equipment type, else Swap if
  `GetCompatibleSlot`; into a container Switch if `CanDrag` and
  `AllowedToAdd`, which asks the container's tech filter and its
  `isAllowedToAdd` delegate, **not its room**.
- **Confirmed (code):** `ExecuteItemAction(action, item)`: Use and Eat
  call `Survival.Use` / `Eat` (the item is removed and destroyed if it
  returns true); Assign begins a quick slot assignment; Equip, Unequip,
  Switch and Swap call `AddOrSwap(item, opposite)`; Drop calls
  `InternalDropItem(item, true)`.
- **Confirmed (code):** `InternalDropItem`: if `CanDropItemHere`, the
  point is `RayCast(MainCameraControl position, forward, 10, 0.75, 1.5)`,
  then `Pickupable.Drop(point, zero, checkPosition true)`; with
  `randomizeRotationWhenDropped` the rotation is `Random.rotation`; then
  `SkyEnvironmentChanged.Send`.
- **Confirmed (code):** `EscapePod.StorageOpen`: nothing while its
  `storageSequence` is active; else `storage_open` true, the open sound,
  and the sequence set for 0.25 s with `StorageOnFlapOpened`, which sets
  the locker as `Inventory.SetUsedStorage` and calls
  `PDA.Open(tab 1, storagePivot, StorageOnClosePDA)`; if that returns
  false, `StorageOnClosePDA` at once. `StorageOnClosePDA`: the sequence
  set for 0.25 s (no callback), the close sound, `storage_open` false.
  `EscapePod.Update` runs the sequence.
- **Confirmed (code):** `PDA.Open` returns false while it is in use (or
  its input is ignored) and in a cinematic; else it returns the held
  tool, sets `isInUse`, `ArmsController.SetUsingPda(true)`, starts its
  0.5 s sequence, hides the crosshair and the view model, and with a
  target stores `activeSqrDistance` = (target − PDA)² + 1.
  `PDA.ManagedUpdate` closes it on `Button.PDA` (when focused, not in the
  intro) and when the target is gone or (target − PDA)² ≥
  `activeSqrDistance`.
- **Confirmed (code, M9u8a1):** "the PDA's place" in both rules is
  `Player.main.transform.position`, the player's position: not a
  stand-in. **Confirmed (real data, M9u8a1):** `EscapePod.storagePivot`
  is the node of the locker's `GenericHandTarget` itself (0.000 m from
  it), not the centre of its collider, which M9d5b used.
- **Confirmed (real data, M9d5b run):** the escape pod's controller has
  the bool `storage_open` (layer "storage": `closed` ↔ `open`, 0.1 s
  transitions). With our stand-ins (the hand collider's centre for
  `storagePivot`, the player's position for the PDA's), the limit where
  the hand opens the locker is 2.75 m.
- **Hypothesis:** `PDA.Open` does not stop the player's movement (the
  distance rule needs it); not checked in the game. M9u8a1 read why: the
  PDA's screen is selected with `lockMovement: false`
  (`uGUI_InputGroup.Select`), so only the mouse look stops.

## The PDA (M9u8a1)

Read from the decompiled `PDA`, `Sequence`, `uGUI_InputGroup`,
`ArmsController`, `MainCameraControl` and `uGUI_CanvasScaler`, a
throwaway probe of the main scene, and `sn-inspect anim scene:main
--states`. Ported in `sn-sim::pda` and `sn-sim::body`.

- **Confirmed (real data):** the PDA is the main scene's
  `Player/body/player_view/export_skeleton/…/hand_L/attachL/PlayerPDA`,
  stored inactive, layer 8. It holds the `PDA` script. Its child `Mesh`
  (not skinned, casts shadows) draws 3 MarmosetUBER materials: `PDA`
  (`MARMO_EMISSION MARMO_SPECMAP _ZWRITE_ON`), `PDA_border` (adds
  `WBOIT _ALPHAPREMULTIPLY_ON _NORMALMAP`) and `PDA_screen`
  (`MARMO_ALPHA MARMO_SIMPLE_GLASS`, premultiplied). Its child
  `ScreenAnchor` (`PDA.screenAnchor`) sits at (−0.205, 0, 0), turned 180°
  about y, scale 0.1 (real-data test `the_pda_in_the_hand`).
- **Confirmed (code):** `PDA.Open` sets the PDA's GameObject active and
  `Deactivated` sets it inactive. While inactive, `PDA.PerformUpdate` only
  runs `UpdateTime`; `ManagedUpdate` (the sequence and the closes) runs
  only while it is active.
- **Confirmed (code):** the sequence is UWE's `Sequence` in floats: `Set`
  starts it from its `t`; each `Update` first checks whether `t` is at the
  target (then it stops and calls back), else moves `t` by ±dT ÷ time,
  clamped. So the callback runs on the update after `t` arrives: opening
  (`timeDraw` 0.5 s, `Activated`) and closing (`timeHolster` 0.3 s,
  `Deactivated`) last their time plus up to two frames at 60 Hz (unit
  tests).
- **Confirmed (code):** `state`: Opening/Opened while the sequence's target
  is true (active or not); Closed when the GameObject is inactive or the
  sequence stopped; else Closing. `isInUse` holds from `Open` to
  `Deactivated`, so `Open` is refused while closing. `Close` does not look
  at the state: called while closing it runs again (sound, callback),
  and the sequence goes on from its `t`.
- **Confirmed (code):** `isFocused` is the screen's input group selected
  with its input dummy on the stack: `Activated` selects it, `OnClosePDA`
  deselects it. So the PDA button cannot close the PDA while it opens.
- **Confirmed (code):** `ArmsController.SetUsingPda` sets `using_pda` in
  `Open` (true) and `Close` (false), so the arms start putting the PDA away
  at once while the model stays until `Deactivated`. With the PDA in use
  `ArmsController` sets no IK target (no tool, and world targets are
  skipped while the PDA is active), so the arms' pose is the animation's.
- **Confirmed (code), the camera's locked mode** (`MainCameraControl.OnUpdate`
  while `PDA.isInUse`): `camRotationX/Y` reset when the mode changes; the
  look's `rotationY` and `camRoot`'s and `cameraUPTransform`'s x angles
  ease to 0 by `Mathf.LerpAngle(…, PDA.deltaTime × 15)`; `camRoot` keeps
  its yaw; the mouse delta (only while the avatar's input is on, i.e.
  before `Activated` selects the screen) goes to `camRotationX/Y`, clamped
  to ±60°, pulled back by `LerpAngle(…, PDA.deltaTime × 10)` and set as
  `cameraOffsetTransform`'s angles (−y, x, 0); out of locked mode those
  angles ease back by `LerpEuler(…, deltaTime × 5)`.
  `cameraOffsetTransform`'s z moves by ±0.25 m/s × `PDA.deltaTime`,
  clamped to `camPDAZStart` … `camPDAZStart + camPDAZOffset` (0.18 m in
  the scene): forward in 0.72 s while in use, back after. No swim bob in
  locked mode. `GetCameraPitch` sums the three transforms' x angles.
- **Confirmed (code):** `MiscSettings.pdaPause` defaults to false, so
  `PDA.deltaTime` is `Time.deltaTime` and nothing pauses.
- **Confirmed (code), the screen's place:** `PDA.ui` instantiates
  `prefabScreen` and calls `uGUI_CanvasScaler.SetAnchor(screenAnchor)`
  (spawn position and rotation identity). In Inversed mode
  `UpdateTransform` puts the canvas at the anchor's world pose taken into
  the main camera's space (`Inverse(cam.rotation) × (anchor − cam)`), for
  the UI camera that sits at the origin, every frame (Inversed stays
  dirty). `UpdateFrustum` is skipped for Inversed with an anchor, so the
  canvas keeps its stored size and scale; the anchor's scale is not used.
- **Confirmed (real data, `anim scene:main --states`):** the arms' PDA
  states are on the `upperbody` layer (4, mask 94 paths): `New State` →
  (`using_pda`, 0.1 s) `first_check` (`player_pda_start`, 0.004 s) → (0 s,
  without `using_tool_first` or `vr_active`) `open` (`player_pda_equip`,
  1.667 s at speed 2) → (exit time 1, 0.1 s) `idle` (`player_pda_idle`,
  looped) → (not `using_pda`, 0 s) `close` (`player_pda_away`, 0.833 s at
  speed 2) → (exit time 0.9, 0.1 s) the machine's exit → `New State`.
  `open_first` (`player_pda_equip_first_V2`) plays with
  `using_tool_first`. The base layer has a `pda` state
  (`player_pda_start`) entered from `Walking` (speed 0.5) and
  `surface swim` only; `Swim` has none. Layer 3 (`both`) has a copy of the
  PDA machine; the only `using_pda` conditions on that layer are inside
  it. **Confirmed (M9u8a2 run):** with empty hands it is not entered
  (layer 3 stays in `nothing` with the PDA open).
- **Confirmed (M9u8a2 run, `sn-inspect walk --pda`, our animator at the
  50 Hz physics step):** on the open, `upperbody` goes `New State` →
  `first_check` → `open` → `idle`, the last transition starting 0.980 s
  after the open (0.1 s + 0.833 s + the steps); on the close `idle` →
  `close` → `New State`, starting 0.400 s after (0.375 s + steps). The
  base layer goes `Walking` → `pda` → `Walking` in the pod and stays in
  `Swim` under water; `right arm`, `left` and `both` stay in `Nothing`,
  `none`, `nothing`. With the arms in `idle`, the camera level and
  `cameraOffsetTransform` 0.18 m forward, `ScreenAnchor` sits at
  (0.0217, −0.0012, 0.1874) m in the eye's space, its +z 0.89° from the
  eye's: the screen faces the camera, about 19 cm in front of it (the
  same in the pod and under water). *Hypothesis:* `PDA.PerformUpdate`
  runs before the camera in a frame (`ManagedUpdate` queue order not
  read).
- **Confirmed (code, M9u8a3):** `GUIHand.OnUpdate` opens the PDA on
  `Button.PDA` (Tab) while the avatar's input is on (not focused, not in
  the intro), and `PDA.ManagedUpdate` closes it with the same button once
  focused, so a press while it opens or closes does nothing. `Activated`
  frees the cursor (`lockCursor = false`); `OnClosePDA`'s `Deselect`
  restores it. The HUD's checks (`uGUI_HealthBar` numbers,
  `uGUI_DepthCompass`) read `isInUse`. `Player.OnKill` closes an opening
  or open PDA once (input ignore off for that call), then ignores PDA
  input until `ResetPlayerOnDeath` clears it. The game's Escape does not
  close the PDA (it deselects the screen and opens the menu); ours
  closes it (no menu yet).

## Death drops (M9d6a)

- **Confirmed (code, `sn-inspect code --il 'Player/<ResetPlayerOnDeath>d__302'
  MoveNext` and `code --il Player OnKill`):** `Player.OnKill` turns the
  player's input and controller off, ignores quick slot and PDA input
  (`SetIgnorePDAInput(true)`), freezes the stats, and starts
  `ResetPlayerOnDeath(5)`. That coroutine waits 5 s, ends the game in
  permadeath, triggers `playerDeathEvent`, sends
  `DisableHeadCameraController`, rebinds the animator (unless piloting),
  shows the respawn screen, `ToNormalMode(false)`, then
  `Inventory.LoseItems()`, then `MovePlayerToRespawnPoint`; 1 s later
  it waits for the world to settle, hides the screen, resets health and
  oxygen.
- **Confirmed (code):** `LoseItems` lists the grid's items then the
  equipment's with `destroyOnDeath`, and calls `InternalDropItem(item,
  false)` for all but the last (see § Inventory), so each drop draws its
  own ray and rotation from the same camera.
- **Confirmed (real data, M9d6a run):** the six supplies, all
  `destroyOnDeath`: a death outside drops five, keeps the last in the
  grid's order (a Nutrient block); a death in the lifepod drops none.

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
  holds one JSON file per language.
- **Confirmed (code, M9d1):** `Language.LoadLanguageFile` reads
  `<language>.json` with LitJson into one object of key → string, and on
  consoles copies keys ending in a platform suffix (`.PS4`, …) over the
  plain key; on desktop nothing is remapped. A tech type's text key is
  its `TechType` name; "Pick up X" is the `PickUpFormat` key formatted
  with that text (`LanguageCache.GetPickupText`).
- **Confirmed (real data):** `English.json` starts with `//` comments
  (our JSON reader skips them) and parses to 3,818 string keys.

## Crafting (M9e1)

**Confirmed by direct reading** of Crafter, GhostCrafter,
CrafterLogic, Fabricator, KnownTech, Inventory and TechData in
the local ILSpy decompile (2026-10-27; not ields.py):

- The inheritance is Fabricator → GhostCrafter → Crafter → HandTarget.
  GhostCrafter selects the Fabricator tree; hand text uses handOverText
  (initializer UseFabricator). Its range uses the player's transform,
  strictly less than closeDistance (initializer 3 m). Serialized values
  override initializers; actual pod values are not measured in M9e1.
- The UI checks known blueprints and ingredients. Survival ingredients
  come exclusively from Inventory.container, not equipment or nearby
  storage. Consumption calls DestroyItem in recipe order, which removes
  the last item or emptiest trailing battery (the existing container rule).
- GhostCrafter spends 5 energy, regardless of TechData.energyCost. It
  consumes power before resources; a direct invalid call can therefore
  spend power before reporting missing ingredients. Our public rule
  validates the UI conditions first, so a refused request is atomic.
- TechData.GetCraftTime returns false when the key is absent (its out
  value is defaultCraftTime, 0). GhostCrafter then uses the sum of its
  spawn animation delay 1 s and duration 1.7 s, **2.7 s**, not Crafter's
  generic 3 s. Explicit times are clamped to at least 2.7 s. CrafterLogic
  adds **0.1 s** to the end time: completion takes 2.8 s by default.
- CrafterLogic starts with craftAmount primary outputs, then one output
  for each linkedItems entry, in order (including duplicates). A zero
  primary amount can still produce linked items. Room is **not** checked
  at craft start; pickup checks each item separately. When room runs out,
  the primary count/linked index remains in the machine for later pickup.
  This settles and corrects the plan's proposed up-front room check.
- On completion GhostCrafter attempts pickup automatically if the player
  is within closeDistance or pickupOutOfRange is true. Otherwise it
  posts CraftingEnd. Clicking the finished result retries pickup. A busy
  hand click does nothing: there is no busy refusal text in this code;
  the reticle shows the recipe's name and progress.
- Fabricator drives the animator bools open_fabricator and fabricating.
  Beams, light and sparks start after the spawn animation delay. The ghost
  is based on the Constructable model or VFXFabricating child, positioned
  at itemSpawnPoint with the VFX offsets/scale, using FX_BUILDING.
- Crafter closes the menu for duration > 5.9 s, otherwise locks it (unless
  Sprint is held). A craft longer than 20 s posts CraftingBegin. Leaving
  range closes the menu but keeps the machine open while an output exists.
- KnownTech initializes from PDAData.defaultTech using AddRange (which
  analyzes entries too). Add recursively analyzes unlocks and updates
  compound dependencies; unread blueprints use Blueprints (3 s), CraftTree
  and Builder notifications. Existing sn-sim KnownTech does not yet model
  compound dependencies.
- Pod power sources find their ancestor relay. PowerSource stores power
  and maxPower; RegeneratePowerSource adds regenerationAmount every
  regenerationInterval while below regenerationThreshhold. The pod's
  serialized power values and repair interactions are **not tested** here.

**Hypothesis:** exact Unity frame/coroutine scheduling at completion;
comparison with the original running game is **not tested**.

## Survival: drain, eating, first aid and HUD (M9s research, 2026-10-27)

These sections are research, **not implemented or tested gameplay**. Evidence
is direct reading of the local full ILSpy output in
`D:/subnautica-rs/out/decompiled/full` (read only), not `fields.py`:
properties are not serialized fields. **Confirmed (source)** means a method
or declaration confirms the rule; it does not claim a runtime prefab census
or comparison with the original running game. All descriptions are our own
prose. Numbers below describe rules, not a shipped balance table.

- **Confirmed (source: `Survival.Start`, `UpdateHunger`, `UpdateStats`,
  `ResetStats`; `SurvivalConstants`):** hunger is scheduled immediately,
  then every 10 s, only when survival is required and stats are not frozen.
  Food loses 100 units per 2,520 s; water loses 100 per 1,800 s. Each
  interval clamps food to 0–200 and water to 0–100. For each stat, the
  shortfall below zero contributes 25 damage per missing unit, summed
  before applying Starve damage. Both empty for one 10 s interval therefore
  produce approximately 23.809524 damage, not two fixed damage amounts.
  After draining, food + water at least 150 heals at 1/24 health per second
  of that interval. Reset/respawn starts food at 50.5 and water at 90.5.
  Temporary-damage healing costs 0.25 food per health healed, clamped.
- **Confirmed (source: `Survival.UpdateWarningSounds`):** warning selection
  prioritizes crossing zero, then crossing strictly below 10, then strictly
  below 20. Only one warning per stat per update; equality at 10/20 has not
  yet crossed. Notification references are serialized, not text to copy.
- **Confirmed (source: `Survival.Eat`; `Inventory.ExecuteItemAction`):** an
  `Eatable` with either current value nonzero counts as successful use.
  Food changes only if current food is at most 99, and can overfill to 200;
  water always changes and clamps to 100. Nonzero values still fire eat/
  drink events when clamping or the food gate prevents a stat increase.
  Negative values can reduce stats. Success removes/destroys that exact
  inventory item; two zero values leave it alone. An edible can consequently
  be consumed at full stats. Bladderfish additionally adds 15 oxygen on
  successful eating. `allowOverfill` and stomach fields do not control this
  method: do not infer mechanics from their names.
- **Confirmed (source: `Inventory.GetAllItemActions`, `GetItemAction`,
  `ExecuteItemAction`):** the formerly unidentified component in the
  inventory-panel section is `Eatable`. Eating is offered from the player's
  grid when equipment is the opposite container, subject to the mode gate
  (Bladderfish has the oxygen exception). Use precedes Eat, then equipment
  actions for the ordinary inventory click. A storage view can change the
  opposite-container choice; do not make every click consume an item.
- **Confirmed (source: `Survival.Use`):** FirstAidKit is handled here;
  there is no separate `FirstAidKit.cs` in this decompile. It requests 50
  health and succeeds only if actual healing exceeds 0.1. Otherwise it
  reports the localized full-health message and retains the item. Tiny
  positive healing at or below the threshold can occur without consumption.
- **Confirmed (source: `Survival.GetWeaknessSpeedScalar`, full-source
  call-site search):** a weakness scalar subtracts 0.02 for each food/water
  unit below 20, but no caller was found in this decompile. **Hypothesis:**
  this is unused legacy behaviour; do not silently introduce a hunger speed
  penalty without proving an actual call path.
- **Confirmed (source: `uGUI_FoodBar`, `uGUI_WaterBar`):** both use capacity
  100 for display, smooth the ratio with PDA delta time and serialized
  damping, then ceil the smoothed value for the number. They subscribe to
  eat/drink events for a 2.5 s punch whose scale depends on the event value;
  PDA use flips the icon to numbers. Pulse curves and spring parameters
  come from the UI asset. Food overfill is not permission to clamp the
  simulation to the HUD's reference capacity. M9u already provides the
  drawing/animation foundation; M9s must feed live values and events.

## Eatable: individual age and decay (M9s research)

- **Confirmed (source: `Eatable` declarations, getters, `SetDecomposes`):**
  base food/water, `decomposes`, decay rate, despawn settings and delay are
  prefab fields. Current food and water each lose elapsed game time since
  `timeDecayStart` times `kDecayRate`, independently floored at −25. Turning
  decomposition off clears its start time; enabling it sets the start only
  if zero. The timestamp is saved per instance. Storage must not replace
  this with a fresh item's age on moves, drops or rejoining.
- **Confirmed (source: `Eatable.GetSecondaryTooltip`, `IsRotten`):** rotten
  requires both current values negative; otherwise freshness labels depend
  on food falling below half or nine tenths of its base value. These labels
  are language keys. No item-value table belongs in source or this document.
- **Confirmed (source: `Eatable.SetDecomposes`, `IterateDespawn`):** enabling
  decomposition schedules checks every 5 s after an initial random delay
  of 0–5 s. Notably `SetDecomposes(true)` schedules them without checking
  `despawns`. A rotten inactive object resets the active tracking flag;
  a subsequently active rotten object starts a timer and is destroyed only
  after elapsed active time is strictly greater than `despawnDelay`
  (initializer 300 s). **Hypothesis:** normal stored items are inactive;
  verify their lifecycle before reproducing world despawn versus inventory
  retention. Moving an item must preserve decay even when despawn pauses.

## Quick slots and the held-tool flow (M9s research)

- **Confirmed (source: `Inventory.Awake`, `InventoryItem.isBindable`,
  `QuickSlots.OnAddItem`):** the player constructs five slots; the six
  strings in `slotNames` are not the slot count. Bindability is TechData
  equipment type Hand. Automatic binding skips scrap metal, requires a
  tool or eligible Use/Eat action and the first copy of its tech type, and
  uses the first empty slot. Its Eat branch excludes Plantable objects and
  objects without LiveMixin. Automatic selection also needs no held item
  and enabled avatar input. Manual binding is a different path.
- **Confirmed (source: `QuickSlots.Bind`, `SlotKeyDown`, `Select`,
  `SlotNext`, `SlotPrevious`, `OnRemoveItem`, `TryRefill`):** bindings refer
  to individual inventory objects. Rebinding the same item to its own slot
  unbinds it; moving it to another clears the old slot. With the PDA in use,
  a slot key binds the hovered eligible item; otherwise it selects/toggles
  a desired slot. Scroll selects selectable slot types and deselects at an
  end, rather than wrapping continuously. Removal unbinds and attempts a
  replacement of the same tech type from unbound grid items on update.
  Save/restore matches item identities, not just tech types.
- **Confirmed (source: `QuickSlots.UpdateState`, `GetTransitionTime`,
  `PlayerTool`):** None → Draw → Hold and Holster → None use a Sequence;
  a removal also enters Drop. Draw may be reversed by a new desired slot;
  held-tool use and bashing postpone holstering. Per-tool draw, holster,
  drop and bash times are prefab fields, with code initializers 0.5,
  0.35, 1 and 0.7 s. Transition times clamp to 0–10 s. Suspension for the
  PDA must not duplicate or lose the held inventory object.
- **Confirmed (source: `QuickSlots.DrawAsTool`, `HolsterAsTool`,
  `PlayerTool.OnDraw`, `OnHolster`):** tools plug into their hand or camera
  socket, use the view-model layer, disable their main collider and become
  kinematic while drawn; holstering restores storage parent/layer/collider.
  Animator holding flags use the tool name. First-use state is per player
  and tech type, with tool flags and animation events controlling visibility
  and IK targets. This is more than drawing another world model at the eye.
- **Confirmed (source: `GUIHand.OnUpdate`, `OnToolUseAnim`,
  `ArmsController.OnToolUseAnim`, `PlayerTool` input methods):** the hand
  dispatches tool input and inventory use while interaction is allowed;
  world actions are suppressed with the PDA. RightHand can execute Use/Eat
  for held ordinary items; live non-plant edibles take a different hand
  path, so inventory eating and held eating are not interchangeable.
  Tool input drives animation; its use event performs the knife action.
  `GUIHand` rejects use-animation events until strictly more than 0.5 s
  since the preceding one. The server must validate the action sequence;
  an arbitrary client animation callback cannot authorize extra damage.

## Knife damage and harvesting (M9s research)

- **Confirmed (source: `Knife.OnToolUseAnim`, `IsValidTarget`):** knife
  damage and attack distance are serialized fields (initializers 25 and
  2 m); damage type is also a prefab field. The forward trace can fall
  back to the most recent interaction volume. A LiveMixin target is rejected
  if weldable, not knifeable, or an escape pod. No LiveMixin permits surface
  feedback, but grants no damage harvest in this method.
- **Confirmed (source: `Knife.GiveResourceOnDamage`; `TechData` harvest
  getters; `HarvestType`):** DamageAlive harvest requires alive before
  damage; its killing hit also adds `harvestFinalCutBonus`. DamageDead
  requires dead after damage, including a killing hit. These are distinct
  from Break, Pick and Click harvest types. Output and bonus come from
  TechData, with DLL defaults when missing. No output for None; ordinary
  output count is one. Inventory addition sets `spawnIfCantAdd` false:
  insufficient room does not justify spawning loose bonus resources.
  **Hypothesis:** exact failed-add feedback and whether subsequent dead
  hits find a despawned target need a lifecycle test per harvestable prefab.
- **Confirmed (source: `BreakableResource.OnHandClick`, `Player.PlayBash`):**
  an outcrop with a bash-capable held tool starts the bash rather than the
  empty-hand hit path. M9d's outcrop use must remain connected to the tool
  animation path and must not apply both hits for one input.

## Tanks, fins and batteries (M9s research)

- **Confirmed (source: `Oxygen`, `OxygenManager`):** tank capacity is
  `Oxygen.oxygenCapacity` on each prefab, not TechData or a tank-name
  constant. Available oxygen is individual saved state; a negative loaded
  value is repaired to full, while zero is not. The player's source is
  registered at start, other sources on equip and unregistered on unequip.
  Totals count breathable registered sources. Sources sort player first,
  other breathable next, non-breathable last; filling walks forward,
  draining walks backward and drains breathable sources only. Thus a tank
  normally empties before the player's reserve. Equal-priority ordering
  is not proved stable; do not invent an ordering rule for multiple tanks.
- **Confirmed (source: `Inventory.ChangeOxygen`, `TryUpdateOxygen`):** grid
  removal unregisters that object's source, and grid additions re-register
  the equipped tank if present. A spare in the grid is not automatically
  another breathing reserve. Fill rate is the DLL's 30 units/s and the
  previously documented surface/cinematic gates still apply. **Hypothesis:**
  prefab tank capacities and initial availability for picked, crafted and
  loaded tanks remain to be measured; no capacity catalogue was extracted
  in this research. Existing player capacity 45 is confirmed by P0, above.
- **Confirmed (source: `Fins`, `UnderwaterMotor.AlterMaxSpeed`):** Fins only
  tracks equipment state; the motor applies equipped fins' speed bonus
  (ordinary fins +1.9, ultra glide +3.2, skipped in Seaglide mode). Before
  that, tank penalties differ by equipped type, spare high-capacity tanks
  have their own penalty, and a minimum speed is applied; a held tool then
  subtracts 1 outside Seaglide mode. Surface multiplier is applied later.
  These modifiers are DLL rules, not TechData. Preserve operation order
  and read each coefficient at runtime rather than copying a balance list.
- **Confirmed (source: `UpdateSwimCharge.FixedUpdate`):** equipped charging
  fins, underwater state and actual velocity strictly greater than 2 m/s
  permit charging the held tool's EnergyMixin. `chargePerSecond` is a
  serialized field, initializer 0.25. Speed equality does not charge;
  neither does swimming with the tool holstered. The charging loop sound
  depends on actual energy accepted, not merely satisfying the motion gate.
- **Confirmed (source: `Battery` property bodies and serialization hooks):**
  `_capacity` is serialized (initializer 100), `_charge` is instance saved
  state. Unity deserialization initializes charge to capacity; protobuf
  restores charge separately. The setter caps at capacity, not at zero.
  Do not mistake the `charge`/`capacity` properties for serialized fields
  or use prefab initialization to refill a restored used battery.
- **Confirmed (source: `EnergyMixin` properties, `ModifyCharge`,
  `ConsumeEnergy`, `OnCraftEnd`, `InitiateReload`, `Select`):** a tool has
  an actual battery in a StorageSlot, with prefab default type, normalized
  default charge, compatibility and replacement permission. Crafting spawns
  a default battery only if absent. Accepted additions/removals clamp to
  room/available charge; ConsumeEnergy succeeds for any positive amount
  removed, even less than requested. The tool-specific caller determines
  cost; there is no universal passive drain here. Reload offers compatible
  grid batteries and the stored battery, including unload. Replacement
  returns the previous battery through ForcePickup; full-grid behaviour
  must follow that existing pending-item path without copying items.
- **Confirmed (source: `EnergyMixin.charge`, `ModifyCharge`):** effective
  charge is zero while electronics are disabled, with a two-second recovery
  ramp on re-enable; underlying battery state is separate. **Hypothesis:**
  disable effects are outside the initial survival slice; retain this as
  an explicit deferred behaviour rather than treating it as ordinary drain.

## Medical cabinets and supply crates (M9s research)

- **Confirmed (source: `MedicalCabinet.Start`, `Update`, `OnHandClick`):**
  cabinet readiness and next-spawn game time are saved state. The prefab
  supplies interval (initializer 600 s), starting-kit flag and construction
  reference. On first start the deadline is now or now + interval; readiness
  uses strictly greater than the deadline. First click opens a ready closed
  cabinet; taking from an open ready cabinet requires 1×1 room, adds one
  kit, schedules a new deadline and toggles the door after 2 s. Full-grid
  clicks can toggle a ready door without taking the kit. Empty cabinets
  show progress and do not toggle on click. **Hypothesis:** actual pod
  interval, initial flag and construction wiring need a serialized census;
  the initializer alone does not prove the pod's balance.
- **Confirmed (source: `SupplyCrate`):** this is not an ItemsContainer.
  It searches for a child Pickupable. A closed unsealed crate opens; an
  open crate tries Pickup on its child. A sealed crate refuses; its sealed
  opened event toggles the lid. Toggling updates `open` only if an Animation
  component exists, and opening activates `setActiveOnOpen`. Open is saved;
  loading an open crate snaps the animation. An empty open crate offers no
  take action; the public click path does not offer an ordinary close.
  Pickup failure clears the cached child reference but the next hover/click
  searches again, so a full inventory must not permanently erase its item.
- **Confirmed (source: `LootSpawner.GetSupplyTechTypes`, `SpawnStoredLoot`):**
  randomized storage loot is a separate mechanism: a serialized list and
  requested count select distinct list indices; the count is capped by list
  length. SpawnStoredLoot derives count from serialized bounds and a sampled
  curve, fills a StorageContainer once while room remains, then saves its
  `lootSpawned` flag. **Hypothesis:** which placed crates use child items,
  placeholder activation or SpawnStoredLoot, and the population of each,
  must be traced in prefab data. Do not put random storage loot into every
  SupplyCrate merely because both use the word supply.


- **Confirmed (source: `TooltipFactory.GetBarValue`, `uGUI_ItemsContainer.DoUpdate`):**
  icon bars prioritize EnergyMixin, then a battery, then Eatable, then an
  oxygen source. Battery/oxygen use available divided by capacity; decay
  uses nonnegative current food/water divided by positive base values,
  combining both when both bases are positive. Non-decomposing food has no
  decay bar. Inventory flags can hide bars; values are refreshed as items
  age. Tooltips read the same individual components, not tech-type averages.

## M9s runtime provenance and remaining measurements

- **Confirmed (source locations above; existing P0/P1 readers):** TechData
  supplies harvest/slot/equipment rules, sizes, recipes and declared energy
  costs; its missing-key defaults and enum identities come from the DLL.
  Prefabs supply Eatable values/rates, Oxygen capacities, Battery capacity,
  EnergyMixin battery policy, PlayerTool timings/targets, Knife damage/range,
  cabinet settings and crate references. Main-scene components supply player
  and charging-fin fields; UI prefabs supply bar curves and presentation.
  Read serialized fields before applying code defaults; properties are not
  fields, and NonSerialized/ProtoMember state is not a prefab balance field.
- **Confirmed (source):** SurvivalConstants metadata contains many constants,
  but compilers inline their uses. Hunger interval, food gate, regeneration,
  first-aid success threshold, negative decay floor, animation guard,
  movement coefficients and surface refill also occur in constructors or
  method IL. M9s must identify each runtime source (metadata constant or a
  bounded method/constructor IL pattern), log provenance and reject missing/
  ambiguous patterns with offsets. Initializers are fallbacks only where
  Unity does not serialize an override. No implementation may use these
  prose numbers as its runtime balance source.
- **Hypothesis / not tested:** original frame scheduling versus the project's
  fixed tick, actual prefab values listed as unmeasured above, loaded-item
  active state, harvest target destruction and crate loot wiring. Runtime
  extraction coverage is **not tested** in this docs-only change. Multiplayer
  ownership, deterministic loot seeds and paused offline save time are our
  server policy, not facts about the original single-player game.
