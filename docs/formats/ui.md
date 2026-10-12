# The game's UI (uGUI, sprites, TextMesh Pro)

Described build: game build `10` (Unity 2019.4.36f1). Readers:
`crates/sn-unity/src/ui.rs`, `crates/sn-assets/src/ui.rs`. Inspector:
`sn-inspect ui` (M9u0), `sn-inspect code --fields <TYPE>`. Plan:
`docs/DESIGN.md` "M9u plan".

Each fact is marked **confirmed** (with how) or **hypothesis**. Only
names, counts and numbers are recorded here, never the game's content.

## Where the UI comes from — confirmed

- **`uGUI.prefab`.** `uGUI.InitializeAsync` (a coroutine,
  `uGUI/<InitializeAsync>d__43.MoveNext`) loads the Addressables key
  `uGUI.prefab` and instantiates it (`UWE.Utils.InstantiateWrap`). If it
  is missing it logs "Cannot find main uGUI prefab…". Confirmed by
  `code --il`. The catalog maps the key to the GameObject at
  `Assets/uGUI/uGUI.prefab` in `allprefabs_asset_ugui.prefab_….bundle`
  (`sn-inspect ui`).
- **The PDA's screen.** The main scene's `PDA` script has two serialized
  fields, `prefabScreen` (GameObject) and `screenAnchor` (Transform).
  The other fields are static, constants, or private without
  `[SerializeField]`: `code --fields PDA`, and the object's 56 bytes end
  at the second field. `prefabScreen` is `uGUI_PDAScreen` in the main
  scene's `sharedAssets` file. `screenAnchor` is a node named
  `ScreenAnchor`. Confirmed by `sn-inspect ui`.
- **Startup spawners** (read while planning, with a throwaway probe):
  `essentials`' `PrefabListSpawner` spawns 21 manager prefabs, among
  them `GUI` and `IngameMenu`. The main scene's `SystemsSpawner` spawns
  `Systems`. Neither is `uGUI.prefab`. *Hypothesis*: one of them starts
  `uGUI.InitializeAsync`. Not traced, and not needed to draw it.

## `uGUI.prefab` — confirmed (`sn-inspect ui`)

The bundle's serialized file holds 387 GameObjects, 385 RectTransforms
and 13 canvases. It contains **`uGUI` (361 nodes, 360 RectTransforms,
12 canvases)** plus **10 other root prefabs (26 nodes)** that its scripts
instantiate: `RecipeItem`, `Ping`, `RecipeEntry`,
`uGUI_MessageQueueText`, `MessageInstance`, `NotificationLabel`,
`TooltipIconEntry`, `Selector`, `WorldCursor`, `SunbeamSignal`. Within
`uGUI`: 143 `Image`, 101 `TextMeshProUGUI`. With the other roots: 148
and 110, the bundle's totals. Every node is on layer 5 (UI).

Hosts of the scripts M9u ports (paths from the root):

| Script | Object |
|---|---|
| `uGUI`, `ErrorMessage` | `uGUI` |
| `uGUI_SceneHUD`, `uGUI_DepthCompass` | `uGUI/ScreenCanvas/HUD` |
| `uGUI_OxygenBar`, `HealthBar`, `FoodBar`, `WaterBar` | `…/HUD/Content/BarsPanel/<Name>Bar`, each with a `uGUI_CircularBar` on its `Bar` child (a fifth is the intro's skip progress) |
| `HandReticle` | `…/HUD/Content/HandReticle`; 10 `uGUI_HandReticleIcon` under `IconCanvas`: Default, Hand, HandDeny, Scan, Progress, Info, Drill, PackUp, Rename, Interact |
| `uGUI_IconNotifier` | `…/HUD/Content/IconNotifications` |
| `uGUI_PopupNotification` | `…/HUD/Content/PopupNotification` (with `Unlock` and `Journal` children) |
| `uGUI_QuickSlots` | `…/HUD/Content/QuickSlots` |
| `uGUI_PlayerDeath` | `uGUI/ScreenCanvas/PlayerDeathVignette` |
| `uGUI_SceneRespawning` | `uGUI/ScreenCanvas/Respawning` (a `uGUI_Fader` on `RespawningScreen`) |
| `uGUI_Overlays` | `uGUI/OverlayCanvas/Overlays` |
| `uGUI_Tooltip` | `uGUI/Tooltip` |

## The PDA screen — confirmed (`sn-inspect ui`)

`uGUI_PDAScreen`: 402 nodes (379 active), all on layer 5, 2 canvases.
`uGUI_PDA` is on the root. `uGUI_InventoryTab` is on
`Content/InventoryTab`, with 6 `uGUI_ItemsContainer`s:
`InventoryContainer`, `StorageContainer` and `TorpedoContainer1`–`4`.
`uGUI_Equipment` is on `Content/InventoryTab/Equipment`.

## Canvases and scalers — confirmed

**`Canvas` (class 223), 55 bytes:** PPtr game object, `u8` enabled
(align 4), `i32` render mode (0 screen-space overlay, 1 screen-space
camera, 2 world space), PPtr camera, `f32` plane distance, `u8` pixel
perfect, `u8` receives events, `u8` override sorting, `u8` override
pixel perfect, `f32` sorting bucket normalized size, `u32` additional
shader channels, `i32` sorting layer id, `i16` sorting order, `i8`
target display. Every one of the 14 canvases read (12 in `uGUI`, 2 in
the PDA screen) ends at its last byte.

`uGUI`'s canvases are all world space, except `uGUI/SafeZoneMarker`
(inactive, screen-space overlay). Top-level canvases: `CraftingCanvas`,
`ScreenCanvas`, `DepthClear`, `Tooltip`, `OverlayCanvas`. Nested ones:
`ScreenCanvas/Pings`, `ResourceTracker`, `HUD/Content/ErrorMessageCanvas`
(override sorting), `ItemSelector`, `CameraScannerRoom/PingCanvas`,
`ControllerDisconnected` (inactive, override sorting). Sorting layer ids
are hashes; their names (in the tag manager) are not read yet. The PDA
screen's canvas is world space and pixel perfect, with plane distance 1.

**`uGUI_CanvasScaler`** (the game's script, `code --fields`): its
serialized fields are `referenceResolution` (Vector2), `mode`,
`vrMode` (`Mode`: 0 Static, 1 Inversed, 2 Parented, 3 World),
`distance` (f32) and `scaleMode` (0 DontScale, 1 Immediate, 2
Postponed). Every object read ends after `scaleMode`. Values:

| Canvas | Mode (VR mode) | Distance | Reference | Scale |
|---|---|---|---|---|
| `uGUI/CraftingCanvas` | Static (Parented) | 1 m | 1920×1080 | Immediate |
| `uGUI/ScreenCanvas` | Static | 1 m | 1920×1080 | Immediate |
| `uGUI/DepthClear` | Static | 1000 m | 1920×1080 | DontScale |
| `uGUI/OverlayCanvas` | Static | 1 m | 1920×1080 | Immediate |
| `uGUI_PDAScreen` | Inversed | 1 m | 1920×1080 | DontScale |

`UpdateTransform` (IL) places the canvas at the UI camera's position +
forward × `distance` with the camera's rotation, or at an `_anchor`'s
pose. *Hypothesis*: Inversed and Parented are the anchored modes, and
the PDA screen follows `PDA.screenAnchor`. Not read yet (M9u5, M9u8a).

## Sprite atlases — confirmed

**Layouts** (field order from UnityPy's type-tree database for
2019.4.36f1, `out/m9u/oracle/typetree.py`; every object of these classes
in the game parsed to its last byte, `sn-inspect ui --sprites`: 1,190
`Sprite`, 8 `SpriteAtlas`, 2,835 `RectTransform`, 138 `Canvas`, 97
`CanvasGroup`, 2,306 `CanvasRenderer`):

- **`SpriteAtlas` (class 687078895):** `string` name; PPtr[] packed
  sprites; string[] packed sprite names (the same count); the render data
  map: `i32` count, then per entry the key {16-byte GUID, `i64`} and
  `SpriteAtlasData` (80 bytes: PPtr texture, PPtr alpha texture, rect
  `textureRect`, vec2 `textureRectOffset`, vec2 `atlasRectOffset`, vec4
  `uvTransform`, `f32` downscale multiplier, `u32` `settingsRaw`; no
  secondary textures in 2019.4); `string` tag; `bool` is-variant
  (padded).
- **`Sprite` (213):** `string` name; rect `m_Rect`; vec2 offset; vec4
  border (left, bottom, right, top); `f32` pixels per unit; vec2 pivot;
  `u32` extrude; `bool` is-polygon (padded); the render data key {GUID,
  `i64`}; string[] atlas tags; PPtr atlas; `m_RD` (PPtr texture, PPtr
  alpha texture, secondary textures {PPtr, string}[], submeshes (48 bytes
  each), the index buffer `byte[]`, vertex data {`u32` count, channels
  {stream, offset, format, dimension}[], `byte[]` data}, bind poses
  (matrices), rect `textureRect`, vec2 `textureRectOffset`, vec2
  `atlasRectOffset`, `u32` `settingsRaw`, vec4 `uvTransform`, `f32`
  downscale multiplier); the physics shape (vec2[][]); bones (none in
  this game).
- **`RectTransform` (224):** the Transform's fields, then vec2 anchor
  min, anchor max, anchored position, size delta, pivot.
- **`CanvasGroup` (225):** PPtr game object, `u8` enabled (padded), `f32`
  alpha, `bool` interactable, `bool` blocks raycasts, `bool` ignore
  parent groups (padded): 24 bytes.
- **`CanvasRenderer` (222):** PPtr game object, `bool` cull transparent
  mesh.

**Lookup** (`SpriteManager`, confirmed from its IL; `crates/sn-assets/src/sprites.rs`):
`.cctor` loads every atlas with the Addressables label `SpriteAtlases`
and maps `SpriteManager.Group` to atlas names (None `Default`, Item
`Items`, Background `Backgrounds`, Category `Categories`, Log `Log`, Tab
`Tabs`, Pings `Pings`, ItemActions `ItemActions`).
`OnLoadedSpriteAtlases` keys each atlas's sprites by name (the clone's
name without `(Clone)`), ignoring case. `Get(group, name, fallback)`
looks up the group's atlas and otherwise gives the fallback.
`Get(TechType)` is `Get(Item, techType.AsString(false), defaultSprite)`,
and `defaultSprite` is `Default`'s `Unknown`. The stored names are often
lower case (`titanium` for `Titanium`), so the case rule matters. Of 793
`TechType` names, 450 have their own icon and 343 get `Unknown`. Of the
515 tech types in `ItemRules`, the 103 without an icon are almost all
things that are never in an inventory (creatures, plants, fragments,
drillables, base machines).

**Packing** (`settingsRaw`: bit 0 packed, bit 1 mode 0 tight / 1
rectangle, bits 2–5 rotation, bit 6 mesh type): every atlas sprite is
packed, none rotated. `Items`, `Categories`, `Tabs`, `Pings` and
`ItemActions` are tight-packed; `Default`, `Backgrounds` and `Log`
rectangle-packed. Tight packing **trims transparent margins**: a 128×128
sprite (`m_Rect`) is e.g. 90.9×106.9 in the atlas (`textureRect`, with
fractional bounds). So drawing an icon in its rect needs the padding
between the two, as uGUI's `Image` applies it (M9u4). Only `Default`
has a sprite with nine-slice borders.

**Oracle (UnityPy 1.25.4, `out/m9u/oracle/sprites.py`):** the first 20
`Items` sprites by name are equal as `f32` (name, rect, border, pivot,
pixels per unit, packed rect, settings). Both `Items` pages' decoded RGBA
(2048×2048 and 1024×2048, RGBA32) have the same CRC-32 as UnityPy's
(`b4bde8b7`, `d521bff4`).

`atlases_….bundle`: 8 atlases. `Items` packs 470 sprites, `Categories`
29, `Tabs` 12, `Pings` 9, `Log` 7, `Backgrounds` 4, `Default` 2 and
`ItemActions` 2. Every packed sprite resolves to a `Sprite`. The
`Items` sprites are all in the bundle's one external file
(`CAB-ef9560b0…`), the others in the atlas file itself. The 7 pages in
the bundle are RGBA32 (`sactx-<w>x<h>-Uncompressed-<Atlas>-…`). The
`Items` atlas's two pages are in the same external file as its sprites,
in `duplicateassetssorted_assets_bundle93_….bundle` (confirmed M9u2).

## Fonts — confirmed

`fonts_assets_all_….bundle`: 9 `Font`s (`m_Name` first; the object
holds the font file, 30 KB–16 MB), 8 `TMP_FontAsset`s, 1
`TMP_SpriteAsset` (`InputSymbols`, its page 1024×512 Alpha8). Of the
font atlases, only `Aller_Rg Atlas` has pixels (1024×1024 Alpha8). The
others are 0×0 in the file.

**Every font asset is dynamic** (confirmed in M9u1, read with the
layout from `Unity.TextMeshPro.dll`, `sn-inspect ui --layout
TMPro.TMP_FontAsset [<NAME>]`). `m_AtlasPopulationMode` is 1 (dynamic),
`m_AtlasRenderMode` 4165 (SDFAA), `m_AtlasPadding` 9, and the atlas is
1024×1024. The glyph and character tables are empty except in `Aller_Rg
SDF`, which holds 2 glyphs (one is U+2026, "…") and their used
rectangles. TMP makes every other glyph while the game runs, from the
font file (`m_SourceFontFile`, the `Font` object) at the asset's
creation settings. Each asset stores its `m_FaceInfo` (family and style
names, point size, scale, line height, ascent, descent, …): the M9u3
check for our own face metrics.

## Script fields (M9u1) — confirmed

A MonoBehaviour's fields follow its header (game object, enabled,
script, name). Without type trees, their layout comes from the script's
class in the game's assemblies, by Unity 2019.4's rules
(`crates/sn-assets/src/script_layout.rs`). Confirmed by reading
**every MonoBehaviour of the game, 45,137 objects of 1,205 script
classes**, each ending at its last byte (`sn-inspect ui --scripts
--all`). Nine objects also agree, field by field, with our hand-written
readers (`Oxygen`, `MainCameraControl`, `LiveMixin` and its
`LiveMixinData`, `PDAData`, 4 `uGUI_CanvasScaler`s; real-data test
`script_layouts_agree_with_hand_written_readers`).

- **Which fields:** declared order, base classes first, up to (not
  including) `MonoBehaviour` / `ScriptableObject`. Not static, const,
  readonly or `[NonSerialized]`. Public, or with
  `UnityEngine.SerializeField`. No game field uses `[SerializeReference]`
  (it would be an error).
- **Which types:** `bool`, the integer types, `char` (2 bytes), `float`,
  `double`, `string`; enums as their underlying type (a `byte` enum is
  1 byte, then padding); references to `UnityEngine.Object` subclasses
  as PPtrs (12 bytes); `T[]` and `List<T>` as `i32` count + elements
  (not arrays of arrays); `[Serializable]` classes and structs inline,
  if not abstract, an interface, generic or a delegate. Fields of other
  types are not stored. Seen in the game: delegates, `UWE.Event<T>`,
  `MonitoredValue<T>`, `HashSet`, `Dictionary`, `Nullable`, interfaces,
  and classes without `[Serializable]`.
- **Built-in structs** are stored by Unity's own layout, from a table:
  `Vector2/3/4`, `Quaternion`, `Color` (floats), `Color32` (4 bytes),
  `Rect`, `Matrix4x4`, `Bounds`, the `…Int` vectors and rects,
  `LayerMask`, `RectOffset`, `AnimationCurve`, `Gradient`, and
  `GUIStyle`: name, 8 states {PPtr background, colour text} (no scaled
  backgrounds in this build), border, margin, padding and overflow
  offsets, PPtr font, font size, style and alignment, word wrap and
  rich text (2 bytes, then padding), clipping, image position, content
  offset, fixed width and height, stretch width and height (2 bytes,
  then padding).
- **Padding:** a field smaller than 4 bytes (bool, byte, short, char,
  small enums), a string, or an array is followed by padding to the
  next multiple of 4 *in the object*. A string pads also as an array's
  element (`Story.CompoundGoal.preconditions`). Elements of other arrays
  do not pad one by one. Inside the built-in structs, the small fields
  are packed as listed.
- **Depth limit:** nested custom classes stop at depth 7
  (*hypothesis*: no object in the game is that deep, so it is not
  tested).
