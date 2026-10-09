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
- Not read yet: `GroundMotor`'s own fields (`CharacterMotorMovement` and
  others), `OxygenManager`, `Survival` (food and water), the `Player`
  fields after `guiHand`.

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
