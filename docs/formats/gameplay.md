# Gameplay data

Where Subnautica (build 10) keeps the data its gameplay needs, so that we
can read it from the player's install at runtime. Written for Phase E
(`docs/DESIGN.md` § 4.3). Found by reading the game's code with a decompiler
(output kept in the system temp folder, never in the repository) and by
searching the install's files. No game content is reproduced here.

## Recipes and item data: `Balance/TechData`

- **Confirmed (code):** `TechData.Initialize` loads the text asset at the
  resources path `Balance/TechData` (`Resources.Load<TextAsset>`) and parses
  it as JSON (LitJson). The per-entry keys it reads are `techType`,
  `itemSize` (`x`, `y`), `backgroundType`, `equipmentType`, `slotType`,
  `craftTime`, `craftAmount`, `ingredients` (each `techType`, `amount`),
  `linkedItems`, `processed`, `buildable`, `soundPickup`, `soundDrop`,
  `soundUse`, `harvestType`, `harvestOutput`, `harvestFinalCutBonus`,
  `maxCharge`, `energyCost`, `poweredPrefab`.
- **Confirmed (file search, 2026-10-09):** the JSON is inside
  `Subnautica_Data/resources.assets`: 976 occurrences of the key `techType`,
  40 of `craftTime`.
- **Confirmed (code):** `techType` values are integers (`json.GetInt`), the
  numeric values of the `TechType` enum.
- **Confirmed (code):** keys missing from an entry fall back to defaults that
  are written in the code (`TechData.defaults`), not in the JSON. P1 reads
  them from the DLL.
- **Hypothesis:** the JSON's top-level layout (an array or object of
  entries). P0 checks it with a parser test.

## Fabricator menus: `CraftTree` (code only)

- **Confirmed (code):** each crafting machine's menu (fabricator,
  constructor, workbench, vehicle upgrade console, rocket, …) is built in
  C# by nested `new CraftNode(id, TreeAction, TechType)` calls joined by
  `AddNode`. There is no data file for it.
- Plan: read the IL of those methods from the player's
  `Assembly-CSharp.dll` at runtime (DESIGN § 4.3, P1).

## Tech type names: the `TechType` enum (code only)

- **Confirmed (code):** `TechType` is a C# enum (787 members in the
  decompiled source). The JSON uses its numbers; names are needed to match
  the language files and the code's own references.
- Plan: read the enum's fields and their constants from the DLL's metadata
  tables (P1).

## Unlocks: `PDAData`

- **Confirmed (code):** `KnownTech.Initialize(PDAData data)` takes the
  starting blueprints (`defaultTech`), the blueprints unlocked by owning
  others (`compoundTech`) and those unlocked by scanning (`analysisTech`)
  from a `PDAData` object.
- **Hypothesis:** `PDAData` is a serialized ScriptableObject in a bundle or
  `.assets` file. Where it is: not found yet (P0).

## Prefab ↔ tech type: `EntTechData`

- **Confirmed (code):** `CraftData` loads the resource `EntTechData`
  (`Resources.Load<EntTechData>`), a list of entries mapping prefabs to tech
  types.
- **Hypothesis:** its entries are class id + tech type. P0 reads it.

## Player numbers

- **Confirmed (code):** `Oxygen.oxygenCapacity` and `Player.suffocationTime`,
  `Player.suffocationRecoveryTime`, `Player.playerSphereRadius` are public
  (serialized) fields, so their values are stored with the player prefab.
- **Hypothesis:** the player prefab is reached from the main scene or the
  Addressables catalog. Not found yet (P0).
- Not checked: swim speeds (`UnderwaterMotor` and related scripts).

## Text

- **Confirmed (file listing):** `StreamingAssets/SNUnmanagedData/LanguageFiles/`
  holds one JSON file per language. Not opened yet; key format not checked.
