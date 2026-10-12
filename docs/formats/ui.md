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

**`SpriteAtlas` (class 687078895), first fields:** `string` name,
PPtr[] packed sprites, string[] packed sprite names (the same count).
The render data map that follows is not read yet (M9u2).

`atlases_….bundle`: 8 atlases. `Items` packs 470 sprites, `Categories`
29, `Tabs` 12, `Pings` 9, `Log` 7, `Backgrounds` 4, `Default` 2 and
`ItemActions` 2. Every packed sprite resolves to a `Sprite`. The
`Items` sprites are all in the bundle's one external file
(`CAB-ef9560b0…`), the others in the atlas file itself. The 7 pages in
the bundle are RGBA32 (`sactx-<w>x<h>-Uncompressed-<Atlas>-…`); the
`Items` page is not among them. *Hypothesis*: it is in the same
external file as its sprites. Checked in M9u2.

## Fonts — confirmed

`fonts_assets_all_….bundle`: 9 `Font`s (`m_Name` first; the object
holds the font file, 30 KB–16 MB), 8 `TMP_FontAsset`s, 1
`TMP_SpriteAsset` (`InputSymbols`, its page 1024×512 Alpha8). Of the
font atlases, only `Aller_Rg Atlas` has pixels (1024×1024 Alpha8). The
others are 0×0 in the file.

**The font assets are small** (784–1,024 bytes each, `Aller_Rg SDF`
1,024). *Hypothesis*: their glyph and character tables are empty in the
build, and TMP fills every atlas while the game runs, so `Aller_Rg`'s
stored pixels have no glyph rectangles to go with them. This affects
M9u3's planned check against the stored atlas. The layout is read in
M9u1, which settles it.
