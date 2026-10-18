# The game's UI (uGUI, sprites, TextMesh Pro)

Described build: game build `10` (Unity 2019.4.36f1). Readers:
`crates/sn-unity/src/ui.rs`, `crates/sn-unity/src/font.rs`,
`crates/sn-assets/src/ui.rs`, `crates/sn-assets/src/fonts.rs`; fonts and
glyphs: `crates/sn-ui/src/font/`. Inspector: `sn-inspect ui` (M9u0),
`ui --fonts` (M9u3), `sn-inspect code --fields <TYPE>`. Plan:
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

## The PDA's screen on the PDA (M9u8a4a) — confirmed (decompiled code, stored fields, client)

`PDA.ui` instantiates `prefabScreen` (`uGUI_PDAScreen`, 402 nodes) once,
calls `uGUI_CanvasScaler.SetAnchor(screenAnchor)` and `uGUI_PDA.Initialize`.
Our client grafts a copy into the UI scene as `uGUI_PDAScreen(Clone)` and
ports `uGUI_PDA`'s states in `sn_ui::pda`:
- **Stored:** the canvas 1195 × 970 at scale 0.00021181 (0.2531 × 0.2055 m),
  pivot (0.5, 0.5). `Inversed` with an anchor skips `UpdateFrustum`, so it
  keeps that size and scale; its pose each frame is `ScreenAnchor`'s in the
  main camera's space (§ Canvas placement). At rest (the arms in `idle`) that
  is (0.0217, −0.0012, 0.1874) m, its +z 0.89° from the camera's (client and
  `walk --pda` agree to 0.1 mm). Our client applies it in the render pass
  each frame, not through the worker, so the screen does not lag the model.
- **`Awake`:** `content` visible, `SetCanvasVisible(false)` (the canvas
  group's alpha 0; the scaler inactive, the masks disabled).
  **`Initialize`:** every tab's `Close`; the back button inactive;
  `SetTabs(regularTabs)` (Inventory, Journal, Ping, Gallery, Log,
  Encyclopedia). A regular tab's `Open`/`Close` is
  `content.SetVisible(true/false)` (`CanvasGroup` alpha 1/0, interactable
  and raycasts with it); `uGUI_IntroTab` and `uGUI_TimeCapsuleTab` set their
  GameObject active/inactive instead (the intro's `Close` also sets the
  background's alpha to 1).
- **`OnOpenPDA(None)`** (from `PDA.Open`): the canvas visible, `content`
  alpha 1 (not interactable until `OnPDAOpened`), then `OpenTab(tabPrev)`:
  the open tab closed, the new one opened, the toolbar group visible when
  the tab is on the toolbar. **`OnClosePDA`** (from `PDA.Close`): the open
  tab closed, `SetTabs(regularTabs)`, `content` hidden. **`OnPDAClosed`**
  (from `Deactivated`): the canvas invisible.
- **`PDABackground`** (`uGUI_PDA.pdaBackground`): an `Image` with the default
  material, white, alpha 1, sprite `PDABackground` (1195 × 970 RGBA32, its
  own texture): alpha is 0 over most of it (1,079,822 of 1,159,150 texels),
  so it is a glowing frame, not a fill. The screen's dark fill is the PDA
  model's `PDA_screen` glass (`materials.md` § The view model's glass).
- **`uGUI_Toolbar.UpdateCycleBtnText`** (`OnEnable`): with the keyboard as
  the primary device `cycleLeftBtnText` and `cycleRightBtnText` are made
  inactive (stored with "?" texts). Ported for the PDA's toolbar.
- **Not ported yet (M9u8a4b):** the toolbar's icons (made by code),
  `Select` and the tab clicks; the equipment slots' `UI/Circle` (drawn as
  `UI/Default`); the inventory tab's containers (our M9d5b panel stands in,
  M9u8b).

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
pose. Read in M9u5b (§ Canvas placement): Inversed and Parented are the
anchored modes. Inversed takes `_anchor` (the canvas's parent when it is
enabled, or one set by `SetAnchor(transform)`) and turns the pose into
the main camera's space. *Hypothesis*: the PDA screen follows
`PDA.screenAnchor` (M9u8a).

## Canvas placement (M9u5b) — confirmed (decompiled code, `ui --canvas`)

Our port: `crates/sn-ui/src/scaler.rs`; `sn-inspect ui --canvas` runs
the check. Read from the decompiled `uGUI_CanvasScaler`, `SNCameraRoot`,
`ManagedCanvasUpdate`, `GraphicsUtil`, `MainCamera` and `MiscSettings`
(read only, in `out/`).

- **The UI camera** is the first camera whose culling mask has layer 5
  (`ManagedCanvasUpdate.GetUICamera`): `MainCamera (UI)`. Its stored
  numbers: clear flags 4 (nothing), depth 2, mask `0x20`. The main
  camera: clear flags 1, depth 0. `SNCameraRoot.Start` unparents the UI
  camera and puts it at the world origin with no rotation. `Awake` copies
  the main camera's near (0.03) and far (1700) planes and field of view
  to it, and `SyncFieldOfView` keeps both at `MiscSettings.fieldOfView`
  (default 60). So it draws over the main camera's image without
  clearing, and only the `DepthClear` canvas resets the depth (colour
  mask 0, z always, at 1000 m).
- **Static mode** (`UpdateTransform`): position = UI camera position +
  forward × `distance`, rotation = the UI camera's.
- **`UpdateFrustum`** (every mode but Inversed with an anchor). It uses
  the **main** camera's field of view (`MainCamera.camera`) and the
  screen size (`Screen.width`, `height`). The frustum at `distance` is
  `2 · distance · tan(fov / 2)` high, × aspect wide. The factor is
  `min(screen.x ÷ reference.x, screen.y ÷ reference.y)`, × `uiScale`
  unless `DontScale`. The size is the screen ÷ factor
  (`SetSizeWithCurrentAnchors`), and `localScale = (s, s', s)` with
  `s = frustum width ÷ screen width × factor` (`s'` from the height).
  `Canvas.scaleFactor` = factor. So the canvas fills the view exactly.
- **`uiScale`** = `MiscSettings.GetUIScale()`: 1 on desktop (1.4
  handheld). VR (`XRSettings.enabled`) adds 10 % and other paths: not
  ported.

**The run** (`ui --canvas`, 2026-10-13). `uGUI`'s 4 scalers
(`CraftingCanvas`, `ScreenCanvas`, `OverlayCanvas` at 1 m, `DepthClear`
at 1000 m and `DontScale`) are all Static, with pivot (0.5, 0.5). Their
stored rects (1920×1080; `DepthClear` 100×100) and scales (0.005) are
editor values that `UpdateFrustum` overwrites. At 1920×1080, 2560×1440,
1920×1200 and 3440×1440, all 16 placements put the corners on the
screen's corners; the worst is 0.0001 px (expected before the run: ≤ 0.5
px). Canvas sizes: 1920×1080 (16:9 at both sizes, factor 1 and 4/3),
1920×1200, 2580×1080. Laid out and drawn at those sizes, `uGUI` gives
178 draws each, with no non-finite vertex. 1920×1080 and 2560×1440 give
the same hash, as the rule says they must. At 1920×1080, 4 draws reach
past the canvas: the two `PopupNotification` control texts, the
`LoadingArtwork` image and `RadioIcon` (*hypothesis*: their stored,
off-screen or cover-filled states). The PDA screen's scaler is Inversed
and follows an anchor: not placed here (M9u8a).

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
`m_AtlasRenderMode` 4165 (SDFAA). Sampling point size 90, padding 9,
atlas 1024×1024 for all but `NotoSansCJKsc-Regular SDF`: point size 45,
padding 5, atlas 2048×2048, and the only one with
`m_IsMultiAtlasTexturesEnabled` (a new page when one is full). The glyph
and character tables are empty except in `Aller_Rg SDF`, which holds 2
glyphs, U+2026 "…" and U+005F "_", and their used rectangles.
*Hypothesis:* those two are TMP's ellipsis and underline special
characters, which a text looks up when it is first laid out
(`TextMeshProUGUI.SetArraySizes` calls `GetEllipsisSpecialCharacter`,
read in its IL). TMP makes every other glyph while the game runs, from
the font file (`m_SourceFontFile`, the `Font` object) at the asset's
creation settings. Each asset's material has `_GradientScale` = padding
+ 1 (10; 6 for CJK).

The rules below are ours, written from what the stored data shows
(`crates/sn-ui/src/font/`). `sn-inspect ui --fonts` checks them, and so
does the real-data test `fonts_remade_from_the_font_files`.

**`Font` (class 128)** (field order from UnityPy's type-tree database
for 2019.4.36f1; all 9 parse to their last byte): `string` name, `f32`
line spacing, PPtr default material, `f32` font size, PPtr texture
(aligned), `i32` ASCII start offset, `f32` tracking, `i32` character
spacing, `i32` character padding, `i32` convert case, `CharacterInfo[]`
(index, uv rect, vert rect, advance, `bool` flipped aligned: 44 bytes
each), kerning map {`u16`, `u16`, `f32`}[], `f32` pixel scale, `byte[]`
font data (aligned), `f32` ascent, `f32` descent, `u32` default style,
string[] font names, PPtr[] fallback fonts, `i32` rendering mode,
`bool` legacy bounds, `bool` round advance (padding after them is
optional).

**Face metrics** (`m_FaceInfo`): ours from each font file equal the
stored ones within 0.01 in 136 of 136 fields (17 per asset: names,
sizes and lines; the three below excepted). Scale `s = point size /
units per em`, in floating point (not FreeType's 26.6):
- ascent line = `hhea` ascender · s; descent line = descender · s; line
  height = (ascender − descender + line gap) · s;
- underline offset = (`post` underline position − thickness / 2) · s
  (integer halving: FreeType's rule), thickness = `post` thickness · s;
- strikethrough offset = mean line / 2.5, its thickness the
  underline's;
- superscript offset = ascent line, subscript offset = descent line,
  both size 0.5; baseline 0, scale 1, face index 0;
- tab width = the space's advance · s, rounded;
- family and style names from the name table (typographic names 16/17
  first, then 1/2; every game font gives the stored names).
- **Cap and mean lines are hinted** in the game: they are the top of `H`
  and `x` but rounded by the font's hinting (66 where the outline gives
  65.06 for `Aller_Rg`, 64 where it gives 64.25 for Open Sans). Ours are
  unhinted, so 12 of the 24 cap, mean and strikethrough values differ by
  under a pixel. TMP uses the stored `m_FaceInfo` while the game runs,
  so the client uses the stored values too. A font without `H` or `x`
  (Noto Sans Thai) has 0.

**Glyph metrics**: equal the 2 stored glyphs exactly (every field as
`f32`). Unhinted FreeType loading:
- the 16.16 scale is `FT_DivFix(point size · 64, units per em)`; every
  outline point and the advance are scaled with `FT_MulFix` into 26.6
  pixels (each rounds half away from zero: 733 units at 90 px on a
  1,000-unit em is 4,222/64 px, not 4,221.44/64);
- width, height and bearings come from the control box of the scaled
  points (all points, off-curve ones too), not from the `glyf` header's
  box (U+2026's header says 673 units, its outline reaches 670 after
  placement);
- **composite glyphs**: each component is scaled first, then its offset
  is added: the offset in font units scaled with `FT_MulFix` and, with
  `ROUND_XY_TO_GRID`, rounded to a whole pixel. U+2026 is three periods
  at 0, 246 and 492 units; 492 → 2,834/64 → 2,816/64 gives the stored
  width (adding offsets in font units, as `ttf-parser` does, gives a
  width 0.27 px too large);
- the bitmap box is FreeType's: floor of the minimum and ceil of the
  maximum, in pixels.

**Atlas packing** (equal to `Aller_Rg SDF`'s stored glyph, used and
free rectangles): MaxRects, best short side fit. The free list starts as
`(0, 0, width − 1, height − 1)`. A `w × h` glyph takes a used rectangle
of `(w + 2·padding + 1) × (h + 2·padding + 1)`. Its glyph rectangle is
`padding + 1` in from the used rectangle's left and bottom, and its
distance field (`(w + 2·padding) × (h + 2·padding)`) starts 1 texel in.
Atlas rows are bottom first.

**Distance field**: texel = `0.5 − d / (2 · _GradientScale)` on 0–255,
`d` the distance in pixels from the texel's centre to the outline
(positive outside). Ours uses the exact distance to the flattened
outline (inside by non-zero winding). Against the 2 stored glyphs'
4,015 texels (their used rectangles): **mean error 1.04/255**, maximum
13/255, 60% equal. Shifting ours by one texel in any direction makes the
error 4.4–11.4, so the placement is right. *Hypothesis:* the remaining
error is because the game's SDFAA mode works from an anti-aliased
bitmap (as its name says), not the outline. Nothing else in the stored
atlas is non-zero.

**Which font draws a character** (confirmed from the IL of
`TMP_Text.GetTextElement`): the text's font asset, then its fallbacks
(`m_FallbackFontAssetTable`, depth first), then the text's own sprite
asset, then `TMP Settings`' fallback fonts, then its default font, then
its default sprite asset. Otherwise the missing-glyph character.
`TMP Settings` (in `resources.assets`, read by its layout): no default
font, no fallbacks, default sprite asset `EmojiOne` (16 sprites),
missing-glyph character 63 ("?"). `Aller_Rg SDF` falls back to PT Sans,
Noto Sans CJK SC, Noto Sans Thai and Open Sans; `Aller_W_Bd SDF` to PT
Sans Bold, Noto Sans CJK SC, Noto Sans Thai Bold and `Aller_Rg SDF`.

**Text coverage**: the English language file uses 118 distinct
characters that need a glyph. 100 are in `Aller_Rg` (or `Aller_W_Bd`)
and 18 come from Noto Sans CJK SC. None is missing. The in-scope
screens hold 141 texts (132 `Aller_Rg SDF`, 9 `Aller_W_Bd SDF`, none
with its own sprite asset). Their stored strings need no missing glyph,
except 6 placeholders the game always replaces before showing them
(confirmed from the IL): the hand reticle's four texts
(`HandReticle.UpdateText` sets each and shows it only if it is not empty)
and the PDA toolbar's two gamepad labels (`uGUI_Toolbar.OnEnable` →
`UpdateCycleBtnText`: hidden without a gamepad, otherwise
`GameInput.FormatButton`). Their stored strings hold private-use
characters (U+E005, U+E019, U+E03B) that no font or sprite asset has.

**Cost** (ours, release build, this machine): 2.2 ms per glyph (118 in
262 ms), with the brute-force distance to every outline segment. The
English glyphs fill 42% of `Aller_Rg SDF`'s atlas.

## Layout (M9u4a)

Our port is `crates/sn-ui/src/layout/`. Inputs are built by
`crates/sn-assets/src/ui_layout.rs`, and `sn-inspect ui --rects` runs
the check. The rules come from the IL of the game's `UnityEngine.UI.dll`
(`code --il UnityEngine.UI:UnityEngine.UI.<TYPE> <METHOD>`), summarised
in `rebuild.rs`; the code is ours.

**Driven values are saved as 0 — confirmed.** The M9u plan's hypothesis
was that the editor saves the values the layout last computed. It does
not: of the 370 values our layout drives in the in-scope trees (`uGUI`,
its 10 other roots, the PDA screen), **370 are stored as exactly 0**
(anchors (0, 0)–(0, 0), anchored position 0, size delta 0). So the
stored rects confirm *which* values are driven: every one we drive is
zeroed. They say nothing about the values themselves. Those are checked
by hand-computed unit tests, and by eye in M9u5. The other direction is
weaker: 97 active zero-size rects (equal anchors, size 0) are driven by
nothing we port. Many are deliberate zero-size anchors (the bars'
containers, the compass points); others belong to scripts outside the
layout system (Scrollbar handles, `ScrollRect` content). They are listed
by `ui --rects`.

**Who takes part** (confirmed from the DLLs' InterfaceImpl tables,
`sn-inspect code --implements <INTERFACE> [<ASSEMBLY>]`). Unity's
classes: `Image`, TMP text, `LayoutElement`, the layout groups,
`ScrollRect` and the input fields are elements. The groups,
`ContentSizeFitter`, `AspectRatioFitter` and `ScrollRect` are
controllers. In the game's DLL: `uGUI_CircularBar` is a self-controller
(it sets its size to `_size` on both axes;
`HandleLayoutAlongAxis`). `AspectRatioLayoutElement`,
`uGUI_BlueprintEntry`, `uGUI_IconGrid` and `uGUI_TooltipIcon` are
elements; `uGUI_BlueprintProgress` and `uGUI_NotificationLabel` are
ignorers. `FlexibleGridLayout` extends `LayoutGroup`. No game class
declares `ILayoutGroup` directly. Met, enabled and active, but not
ported: `FlexibleGridLayout`, `TMP_InputField` ×2, `uGUI_IconGrid`,
`uGUI_NotificationLabel` and `uGUI_TooltipIcon` (1 each). Their values
count as unknown.

**TMP text as a layout element** (IL of `TMP_Text`): min width and
height 0 (`m_minWidth`, not serialized), flexible −1 (set in `.ctor`),
priority 0, preferred from the text layout (`GetPreferredWidth`, M9u4c).
`ScrollRect`: every value −1, priority −1. `Image`: min 0, flexible −1,
priority 0. Its preferred size is the sprite's rect divided by
`pixelsPerUnit` = sprite pixels per unit / the canvas's reference
pixels per unit (100). For sliced and tiled images it is the borders'
sum instead (`DataUtility.GetMinSize`).

**The run:** 23 layout roots in `uGUI`, 27 in the PDA screen. A second
run from our own result moves nothing. 168 of the 370 values need a
text size or an unported component, so they are unknown until M9u4c.

## Graphic meshes (M9u4b)

Our port is `crates/sn-ui/src/mesh.rs`. The images' fields and sprites
come from `crates/sn-assets/src/ui_layout.rs` (`ui_graphics`,
`sprite_info`), and `sn-inspect ui --meshes` runs the check. The rules
are from the IL of `UnityEngine.UI.dll`:
- **`Image.OnPopulateMesh`**: without a sprite, `Graphic`'s quad (UV
  0–1, the white texture). Otherwise by `type`: simple (the sprite's own
  mesh if `useSpriteMesh`), sliced, tiled, filled.
- **`GetDrawingDimensions`**: the sprite's size is rounded to whole
  pixels (`RoundToInt`), and the rect is inset by the sprite's trimmed
  margin (`DataUtility.GetPadding`) as a share of that size.
  `PreserveSpriteAspectRatio` shrinks the rect around its pivot.
- **Sliced**: 4 × 4 grid points from the padding and the borders
  (divided by `pixelsPerUnit × pixelsPerUnitMultiplier`; pixels per unit
  = the sprite's / 100), 9 quads, the centre left out without
  `fillCenter`. `GetAdjustedBorders` scales both borders of an axis down
  when the rect is smaller than they are together. A sliced image whose
  sprite has no border is drawn simple.
- **Tiled**: a mesh of tiles (the last row and column cut, their UVs
  cut to match) when the sprite has a border, is packed, or its
  texture does not repeat. Otherwise one quad whose UVs are scaled by the
  tile count. The vertex count is capped at 65,000 (tiles enlarged).
- **Filled**: horizontal and vertical fills cut the quad and its UVs.
  Radial 90/180/360 cut 1, 2 or 4 parts with `RadialCut`: fill under
  0.001 draws nothing, and over 0.999 uninverted draws whole. The angle
  is fill × 90°, and odd corners invert the direction.
- **`RawImage`**: one quad with `uvRect`, if it has a texture (without
  one it is the white texture).
- Colours: `Color` → `Color32`, clamped and rounded.

**Sprite data** (`SpriteInfo`): the outer UV is the sprite's
`textureRect` in its texture. That is its own texture, or for a
late-packed sprite (null texture) its atlas's render data. The padding
is `textureRectOffset` and the rest of `rect` around `textureRect`.
*Hypothesis:* the inner UV is the texture rect inset by each border less
the trimmed margin. The game does not test it: no in-scope image uses an
atlas-packed sprite; all draw from their own textures.

**The game's own graphics** (`sn-inspect code --extends Graphic`):
`uGUI_Block`, `uGUI_CircularBar`, `uGUI_Compass`, `uGUI_EquipmentSlot`,
`uGUI_ItemIcon`, `uGUI_ListItem`, `uGUI_Logo` and, extending
`MaskableGraphic`, `uGUI_Icon`. They build their own meshes (M9u6,
M9u8b). `uGUI_TextGradient` is a mesh modifier (`IMeshModifier`), not
a graphic.

**The run** (`ui --meshes`, active and enabled graphics of `uGUI`, its
other roots and the PDA screen): 243 meshes, 2,028 vertices. 174 simple
(696 vertices), 34 sliced (1,192), 16 without a sprite (64), 11 tiled by
texture repeat (44), 1 tiled as quads (24), 1 filled (4), 10 RawImages
(40). They draw from 98 textures. Every vertex is inside its rect,
every UV inside its sprite's rectangle, every index valid. Not built:
33 images whose rects wait for text layout, 115 TMP texts (M9u4c), and
91 of the game's own graphics.

## Text layout (M9u4c)

Our port is `crates/sn-ui/src/text/`. The game's texts are read by
`crates/sn-assets/src/texts.rs`, and `sn-inspect ui --texts` runs the
checks. The rules are from the game's `Unity.TextMeshPro.dll`, read
decompiled with `ilspycmd` into `out/` (never kept in the repository).
The code is ours and follows the same control flow.

**What the screens use** (`ui --texts`, 141 texts, 115 active):
alignment left / centre / right (`HorizontalAlignmentOptions` 1, 2, 4)
and top / middle / bottom / baseline (256, 512, 1024, 2048). 20 texts
auto-size, 67 wrap, 16 have line spacing, 2 have margins, 20 are bold
or italic, 3 have kerning on. Overflow is 0 (overflow) or 3 (truncate).
The stored texts use only `<b>`, `<color>` and `<size>`. The English
file adds `<b>`, `<color>` and the subtitles' `<delay>` / `<duration>`
(not TMP tags: TMP shows them as text).

**Input — confirmed (decompiled):**
- A text stored in a scene has input source `TextInputBox` (the field's
  default), so its escapes (`\n`, `\t`, `\uXXXX`, …) are parsed. One
  set from code (`text = …`) is `TextString`, so they are not.
  `<br>`, `<nbsp>` and `<zwsp>` become characters before layout.
- Tags (`ValidateHtmlTag`): from `<` to the first `>`, no `<` between,
  under 128 characters. The name hash is 7h + c, and the table lists each
  name all lower case and all upper case, so `<size>` and `<SIZE>` are
  tags but `<Size>` is text. An unknown name is text. `<size=N>` is
  pixels (`+N` / `-N` relative to the size), `em` ×, `%`. `</size>` pops
  TMP's stack. `<i angle=N>` sets the slant; `<i=N>` is plain italic.
- A character's font (`GetTextElement`): the text's font with its weight
  table (bold = weight 700; `Aller_Rg SDF` → `Aller_W_Bd SDF`, `PTSans
  SDF` → `PTSans-Bold SDF`, `NotoSansThai-Regular` → `-Bold`), then its
  fallbacks depth first, then `TMP Settings`' fallbacks and default font
  (none in the game). A missing character becomes `?` (the settings'
  missing glyph), then a space, then ETX. Every font asset has empty
  "synthesized" characters for ETX, tab, LF, VT, CR, U+061C, ZWSP,
  LRM, RLM, LS, PS and WJ when its font lacks them.
- Bold from the weight table's face is a real bold: no `boldSpacing`.
  Without one, TMP adds `boldSpacing` (7) × size / 100 to each advance.

**Layout — confirmed (decompiled):** the scale is size / the asset's
point size × the face scale. A character advances by its glyph's advance
(at the sampling size) × scale, plus (spacing offset + character spacing
+ bold spacing) × size / 100, all × (1 − width adjustment); a white
space adds word spacing × size / 100. Lines advance by −descender of the
line + the next line's ascender + (line gap + delta) × scale + line
spacing × size / 100. The line gap is the face's line height less
ascent − descent. Wrapping, overflow and truncation work by saving the
whole state at break points and restoring it (`SaveWordWrappingState`).
Auto-size starts at the base size clamped to the range and steps by
halves of the remaining range, sizes rounded to 0.05, up to 100 passes
(`OnPreRenderCanvas`, `GenerateTextMesh`). The vertical anchor is the
top less the first line's ascender; the middle uses the last visible
descender. Lines align by their `maxAdvance` (the last visible
character's advance less its trailing spacing). Preferred sizes
(`CalculatePreferredValues`) come from a separate, simpler loop: the
width unwrapped at the maximum size, the height wrapped at the rect's
width less margins, both rounded up to the hundredth.

**Runtime kerning — confirmed, not ported:** `TMP Settings` has
`getFontFeaturesAtRuntime` on. So when a glyph is added, the game reads
the font's pair adjustments (`FontEngine.GetGlyphPairAdjustmentRecords`)
into the asset's feature table. That matters to the 3 texts with
kerning on. Our tables are empty (logged); reading the font's kerning
is a later step.

**The checks** (`ui --texts`): **auto-size, 20 of 20 texts reach the
size the editor stored** (`m_fontSize`, while `m_fontSizeBase` keeps
the designer's). Most stay at their base. The non-trivial ones: both
feedback titles (25.3 in 12 passes), the encyclopedia's controls hint
(25.3 in 11 passes, in a rect itself sized by text), and the PDA
toolbar's two labels (40 in 11 passes). 137 texts of the trees lay out
in 12.8 ms. With text sizes, M9u4a's unknown driven values drop from
168 to 19 (the unported components); all 370 are still stored as 0.

**Not ported** (logged where met; none in the in-scope stored texts):
sprites, monospace, right to left, ellipsis, page and linked overflow,
justified and flush alignment, upper / lower / small caps, underline
and strikethrough, the CJK line-breaking rules (every CJK character may
break), `<line-height>` and the other tags. The text's quads and UVs
(with the material padding, `ShaderUtilities.GetPadding`) are built
with the drawing in M9u5.

## Draw order (M9u4d)

Our port is `crates/sn-ui/src/draw.rs`. Its inputs come from
`crates/sn-assets/src/ui_layout.rs` (`ui_draw_parts`), and `sn-inspect
ui --draw` runs the check. Clipping follows the decompiled
`UnityEngine.UI.dll` (`RectMask2D.PerformClipping`,
`Clipping.FindCullAndClipWorldRect`, `MaskUtilities.GetRectMasksForClip`,
`MaskableGraphic.Cull`). Canvas batching is native, so the order follows
Unity's documented rules.

- **Order:** each root canvas draws its graphics depth first in
  hierarchy order. A nested canvas draws in place unless it has
  `overrideSorting`; then it is its own draw group. Groups sort by
  (sorting layer's index in the TagManager, sorting order, hierarchy
  order).
- **Alpha:** `CanvasGroup` alpha multiplies down the hierarchy, stopping
  at `ignoreParentGroups`. A `CanvasRenderer` with `cullTransparentMesh`
  draws nothing at alpha 0.
- **Clipping:** the intersection of every active `RectMask2D` above a
  maskable graphic (itself included; none across an override-sorting
  canvas). Each mask's rect is its corners 0 and 2 in its canvas's
  space, inset by `m_Padding` (left, bottom, right, top). This build's
  `RectMask2D` has `m_Padding` and `m_Softness`. An empty intersection,
  or a graphic whose own rect does not overlap it, is culled.
- **Transforms:** every vertex goes to its root canvas's space through
  each node's local transform (its anchored place, `z`, rotation,
  scale). A plain `Transform` (the `uGUI` root) is a rect of size 0 at
  its position, as Unity gives its children a parent size of 0.

**TagManager sorting layers** (in order): Default, PDA, DepthClear,
EndCredits, XMenu, HUD, CraftingMenu, BuilderMenu, ItemDragManager,
Tooltip, Overlay, StartScreen, ErrorMessage, IngameMenu, … (the names
only; `sn-inspect ui --draw` prints them all).

**The run** (`ui --draw`; images, raw images and the texts' character
quads): `uGUI` gives 174 draws, 3,824 vertices and 57 textures. Its
draw groups, in order: `DepthClear`, `ScreenCanvas` (168), `Tooltip`,
`OverlayCanvas`. One graphic is culled by a mask and one is transparent.
The PDA screen gives 192 draws (2 clipped by its masks, 147 with alpha
below 1: its stored state is faded out). The other roots of `uGUI.prefab`
have no canvas of their own: the game instantiates them under one.
**Built twice from scratch, 12 of 12 hashes are equal.** No vertex is
non-finite.

**Not ported:** the stencil `Mask` (none in scope), `m_Softness`'s edge
fade (kept with the item for the shader, M9u5), world-space canvases'
distance sorting between root canvases (they sort by layer and order
here).

## UI shaders and materials (M9u5 research)

Read on 2026-10-13 with UnityPy (dev-time oracle in system temp, not a
dependency) and `D3DDisassemble` on the D3D11 programs; throwaway
scripts and disassembly in the ignored `out/m9u5/`. *Confirmed* by those
reads unless marked otherwise.

**Where the shaders are.** `uGUI.prefab`'s bundle holds `UI/Simple`,
`UI/Bar`, `uGUI/WorldCursor`, `uGUI/Compass`, `uGUI/DepthClear`,
`UI/ResourcesBlip` and `uGUI/IntroLogo`. The fonts bundle holds
`TextMeshPro/Distance Field` and `…/Mobile/Distance Field`. `UI/Default`
is in `Resources/unity_builtin_extra` (path id 10770), not in `unity
default resources`. Each shader has one sub-shader with one pass.

**Materials in `uGUI.prefab`'s bundle** (name: shader, keywords):
`HUDBackground` and `OxygenBarPulse`: `UI/Simple`, `ALPHA_PREMULTIPLY`;
`UIBarIcon`: `UI/Simple`, none; `LoadingBar`: `UI/Bar`,
`_FILL_HORIZONTAL`; `ExosuitThrust`: `UI/Bar`, `SHEAR _FILL_HORIZONTAL`;
`WorldCursor` and `Selector`: `uGUI/WorldCursor`, `_ALPHAPREMULTIPLY_ON`
(not a keyword of that shader's programs); `compass`: `uGUI/Compass`;
`UIResourcesBlip`: `UI/ResourcesBlip`; `LoadingLogo` and `SavingLogo`:
`uGUI/IntroLogo`. The fonts bundle has one `… Atlas Material` per font
asset (`TextMeshPro/Distance Field`, no keywords), `InputSymbols Outline`
and `InputSymbols Shadow` (`UNDERLAY_ON`), and 9 `Font Material`s on the
built-in font shader (`1:10101`, legacy `Font`s, not TMP). Which graphics
use which material is not counted yet (M9u5a).

**Render state** (the pass's; names are material properties):

| Shader | Blend (colour / alpha) | Z write, test | Cull | Variants (vertex + fragment) |
|---|---|---|---|---|
| `UI/Default` | `SrcAlpha OneMinusSrcAlpha` both | off, `unity_GUIZTestMode` | off | 4 + 4: `UNITY_UI_ALPHACLIP`, `UNITY_UI_CLIP_RECT` |
| `UI/Simple` | `_SrcFactor _DstFactor` / `_SrcFactorA _DstFactorA` (defaults 5, 10) | off, `unity_GUIZTestMode` | `_Cull` | 4 + 4: `ALPHA_PREMULTIPLY`, `UNITY_UI_ALPHACLIP` |
| `UI/Bar` | `SrcAlpha OneMinusSrcAlpha` | off, `unity_GUIZTestMode` | off | 32 + 32: `_FILL_HORIZONTAL`/`_VERTICAL`, `SHEAR`, `SUBDIVIDE`, `USE_BACKGROUND`, `UNITY_UI_ALPHACLIP` |
| `uGUI/WorldCursor` | `SrcAlpha OneMinusSrcAlpha` | off, `unity_GUIZTestMode` | off | 2 + 2: `UNITY_UI_ALPHACLIP` |
| `uGUI/Compass` | `SrcAlpha OneMinusSrcAlpha` | off, `unity_GUIZTestMode` | back | 2 + 2: `UNITY_UI_ALPHACLIP` |
| `TextMeshPro/Distance Field` | `One OneMinusSrcAlpha` (premultiplied) | off, `unity_GUIZTestMode` | `_CullMode` | 8 + 8: `UNDERLAY_ON`, `UNITY_UI_ALPHACLIP`, `UNITY_UI_CLIP_RECT` |
| `uGUI/DepthClear` | colour mask 0 | **on**, always | off | 1 + 1 |

All but `DepthClear` also take the stencil from `_Stencil…` properties
(the stencil `Mask`, none in scope) and the colour mask from
`_ColorMask`.

**What the programs do** (from the disassembly; constant names from the
programs' binding data):
- **`UI/Default`.** Vertex: `unity_ObjectToWorld` then
  `unity_MatrixVP`; colour = vertex colour × `_Color`; UV through
  `_MainTex_ST`; with `UNITY_UI_CLIP_RECT` it passes the vertex position
  ×2 minus the clip rect's corners and the mask softness factor
  `0.25 / (0.25 × _UIMaskSoftness + |pixel size|)`, the pixel size from
  `glstate_matrix_projection` and `_ScreenParams`. Fragment:
  (`_MainTex` + `_TextureSampleAdd`) × colour; with the keyword alpha ×=
  the soft clip factor `saturate((half size − |offset|) × softness)` on
  both axes. So `RectMask2D`'s softness is drawn here.
- **`UI/Simple`.** The same vertex part without UV transform or soft
  clip. Fragment: the hard clip `UnityGet2DClipping` (`_ClipRect`)
  **always**, no keyword; with `ALPHA_PREMULTIPLY` the colour is ×
  alpha.
- **`uGUI/WorldCursor`.** As `UI/Simple`'s fragment without
  premultiplying (its materials' `_ALPHAPREMULTIPLY_ON` selects no
  program).
- **`UI/Bar`** (`_FILL_HORIZONTAL`): discards where the UV's x, rescaled
  by `_TopBorder` and `_BottomBorder`, is above `_Amount`; samples
  `_BarTex`; hard clip. The other variants are not read yet.
- **`uGUI/Compass`.** Samples `_MainTex` at the UV rotated by `_Angle`
  about the centre; alpha × a `sin` window of the UV's polar angle
  between `_AlphaFrom` and `_AlphaTo`; hard clip.
- **`TextMeshPro/Distance Field`.** Vertex: the SDF scale from the
  vertex's `uv2.y` (TMP's per-character scale), `_GradientScale`,
  `_ScaleX/Y`, `_Sharpness`, the screen's pixel size (projection and
  `_ScreenParams`) and `_PerspectiveFilter` (with the view direction and
  the vertex normal); the face and outline thresholds from
  `_FaceDilate`, `_OutlineWidth`, `_OutlineSoftness`, `_WeightNormal` /
  `_WeightBold` (bold when `uv2.y` ≤ 0), `_ScaleRatioA`; `uv2.x` unpacks
  to the face/outline texture UV (÷ 4096, × 1/512). With `UNDERLAY_ON`
  the underlay's UV offset, scale and bias from `_Underlay…` and
  `_ScaleRatioC`. Fragment: the atlas's alpha against the thresholds
  gives face and outline (both textures sampled, scrolled by `_Time`),
  the underlay behind, the soft clip as `UI/Default`'s (keyword
  `UNITY_UI_CLIP_RECT`), × vertex alpha; output premultiplied.

**Hypotheses for M9u5:** uGUI enables `UNITY_UI_CLIP_RECT` only on
graphics under a `RectMask2D` (`MaskableGraphic`'s clip path, to read in
the IL); `_TextureSampleAdd` is (1, 1, 1, 0) for alpha-only textures and
0 otherwise; the UI camera's `unity_GUIZTestMode` is `LessEqual` and
the UI's own order decides what is in front, since no UI pass writes
depth except `DepthClear`; vertex colours reach the shader converted to
linear (the project is linear, `docs/formats/terrain-materials.md`).
**Read in M9u5a:** the clip keyword is not in the IL.
`MaskableGraphic.UpdateClipParent` calls the native
`CanvasRenderer.EnableRectClipping(rect)` and sets `clippingSoftness`, so
the keyword, `_ClipRect` and the softness constants are set natively. It
stays a *hypothesis* (Unity's documented behaviour). The same holds for
`_ClipRect` on graphics with no mask: `UI/Simple` always tests it, and
those graphics show in the game, so it must then cover everything.

**`UI/Circle`** (found by M9u5a's census; in `main.unity`'s bundle,
path id 648; dumped the same way). It is used by the PDA screen's
equipment slots (`Equipment99`, `…124`, `…174`, `ExosuitEquipment99`,
keyword `BORDER`, which is not a keyword of its programs) and
`PDAIntroCircle1–3`. One pass: `SrcAlpha OneMinusSrcAlpha`, no z write,
`unity_GUIZTestMode`, no culling, 2 + 2 variants (`UNITY_UI_ALPHACLIP`).
Its fragment maps the UV to polar coordinates about the centre: u = the
angle ÷ 2π × `_TileX`, and v = `saturate((r − (_Size − 2 ×
_MainTex_TexelSize.w) ÷ _Size) ÷ (1 − that))` × `_TileY`. It samples
`_MainTex` (+ `_TextureSampleAdd`) × colour, and applies the hard clip.

## Materials and text vertices (M9u5a)

Our port: `crates/sn-ui/src/material.rs`, `crates/sn-ui/src/text/mesh.rs`
and `crates/sn-assets/src/ui_draw.rs` (the draw list built with them;
`sn-inspect ui --draw` checks it). Rules ported from the decompiled
`TextMeshProUGUI`, `TMP_Text`, `TMPro.ShaderUtilities` and
`TMP_MaterialManager` (read only, in `out/`):

- **Which material.** An `Image` or `RawImage` uses `m_Material`, or
  `UI/Default` without one. A text uses `m_sharedMaterial` when its
  `_MainTex` is the font asset's atlas, else the font asset's
  `material` (`LoadFontAsset`). Characters from another font asset (a
  fallback, or a weight table's bold face) draw in a sub-mesh. Its
  material is a copy of the text's, with that font's `_MainTex`,
  `_GradientScale`, `_TextureWidth/Height` and `_WeightNormal/Bold`
  (`GetFallbackMaterial`).
- **Padding** (`GetPadding`, after `UpdateShaderRatios` sets
  `_ScaleRatioA/B/C` on the material): the widest reach of dilate +
  outline (+ glow, + underlay per side with `UNDERLAY_ON`), plus 4 with
  `m_enableExtraPadding`, capped at 1, × `_GradientScale`, + 1.25. A
  non-SDF material gives `_Padding` (truncated) + 1 (+ 4 extra).
- **Style padding** per character (`GenerateTextMesh`'s `num7`): bold
  (not from a bold typeface) `boldStyle ÷ 4 × gradient × ratio A`,
  otherwise `normalStyle ÷ 4 × gradient × ratio A`. If the padding and
  style padding together exceed the gradient scale, the padding shrinks
  to fit. The quad grows by the padding on every side and by the style
  padding on the left and right.
- **UVs** (`SaveGlyphVertexInfo`): the glyph rect grown by padding +
  style padding (style padding only for an SDF material, `_WeightNormal`
  present), ÷ the atlas size. Vertex alpha is at most the text colour's.
- **`uv2`** (the alignment pass): x = `PackUV` of the face-texture UV
  (9 bits each, `x · 4096 + y`; `Character` mapping: corners (0, 0),
  (0, 1), (1, 1), (1, 0)), y = the character's scale × `(1 −
  charWidthAdjDelta)`, negative for bold, × `|lossyScale.y|`. Ours
  holds the scale up to the root canvas. The canvas's own scale (set
  when the game runs) is multiplied in when drawn.

**The run** (`ui --draw`, 2026-10-13). Every draw item has a material:
372 of 372. Draws per shader (keywords, with the clip keyword):
`UI/Default` 208, `TextMeshPro/Distance Field` 96 + 7 `UNDERLAY_ON`
(`Aller_Rg Outline` and `Shadow`) + 2 `UNITY_UI_CLIP_RECT`, `UI/Simple`
3 + 9 `ALPHA_PREMULTIPLY`, `UI/Bar` 2 (`ExosuitThrust`, `LoadingBar`),
`uGUI/WorldCursor` 1, `UI/Circle` 43 (PDA screen only), `uGUI/DepthClear`
1. The 4 TMP materials save every property the programs read. Their
ratio A is 0.9 (gradient 10), so their padding is 1.25 texels, below the
atlas padding of 9. Texts: `uGUI` 97 with 7 sub-meshes, the PDA screen 31
with 1. Every sub-mesh is `Aller_W_Bd` inside an `Aller_Rg` text (bold).
817 text quads: 0 UVs outside the atlas, 0 centres outside a packed
glyph, 0 reaching past their packed rect. **Glyphs: 110 made in 0.325 s**
(2.96 ms each; target ≤ 1 s): `Aller_Rg SDF` 69, `Aller_W_Bd SDF` 41.
No atlas filled up. Two runs from scratch: 12 of 12 hashes equal.
Nothing unported was met (texture mappings other than `Character`, and
vertex gradients).

**Differences from the game:** glyphs are added in tree order, so their
atlas places differ from the game's (the UVs follow ours). A sub-mesh
draws right after its text. In the game it is a child object
(`TMP_SubMeshUI`) after the text's other children; none of the in-scope
texts with sub-meshes has children (*not checked one by one*).

## Drawing in the client (M9u5c)

Our port: `apps/sn-client/src/game_ui.rs` and `game_ui.wgsl`, behind
`--game-ui`.
- **The pass.** It runs after tonemapping, onto the view's main texture
  (load, no clear, no depth), in the draw list's order, one draw call per
  item. This matches the game's UI camera: depth 2, clear flags
  "nothing", drawn after the main camera (§ Canvas placement).
- **Projection.** The UI camera at the origin with Unity's axes and the
  main camera's field of view. Each item's root canvas is mapped by its
  scaler's placement.
- **Textures.** Decoded to RGBA8 and stored bottom row first, so
  Unity's UVs sample them as they are. They are uploaded sRGB when the
  texture's colour space is 1, with its filter and wrap, without mips.
  `_TextureSampleAdd` is (1, 1, 1, 0) for `Alpha8` textures
  (*hypothesis*), else 0.
- **Colours.** Vertex colours are made linear from their stored bytes,
  and `_Color` likewise (*hypothesis*, as in § UI shaders).
- **Shaders.** `UI/Default` (soft clip with `UNITY_UI_CLIP_RECT`) and
  `UI/Simple` (hard clip always, `ALPHA_PREMULTIPLY`, blend factors from
  the material) are ported. `uGUI/DepthClear` is skipped: it only writes
  depth, which our pass does not use. `UI/Bar` is drawn as `UI/Default`
  and named in the log. Texts wait for M9u5d.

### Start-up state (M9u5c) — confirmed (decompiled code)

The stored `uGUI.prefab` is not what the game shows: four full-screen
opaque black panels are active and opaque in it (`Intro/Fader`,
`Respawning/RespawningScreen`, `HardcoreGameOver/InputBlocker`,
`OverlayCanvas/Overlays/OutOfOxygen`), and the first client run drew a
black screen. The game's scripts hide them as it starts. Our port is
`crates/sn-assets/src/ui_start.rs` (their start only; their animations
and later calls are M9u6):
- `uGUI_Fader.Awake` calls `SetState(initialState)`, and
  `Sequence.ForceState` makes the sequence active. Its first `Update`
  (`ApplyState`) sets `canvasGroup.alpha = 0.5 (1 − cos π t)`, so 1 or 0.
- `uGUI_SceneIntro.Start` calls `fader.SetState(enabled)` then
  `fader.FadeOut()`. Settled after the fader's `fadeInTime`: alpha 0.
- `uGUI_Overlays.Awake` calls `SetAlpha(overlay, 0)` on each overlay:
  the graphic is disabled when its curve at 0 is not above 0.
- `uGUI_HardcoreGameOver.Awake` makes `inputBlocker` and `message`
  inactive.

Run: 3 faders to alpha 0 (intro, loading screen, respawning), the
out-of-oxygen overlay disabled, the game-over blocker and message
inactive. Afterwards, no full-canvas draw has alpha above 0.

**The other screens (M9u6s) — confirmed (decompiled code, `ui --start`).**
With only the rules above, the start state still drew 151 visible draws
in 8 screens: the feedback panel and its smileys, the confirmation
dialog, the radiation and notification pop-ups, the vehicles' and
cameras' screens, every hand reticle icon and text at once, the compass,
the build watermark, the tooltip. Each one's script hides it at start.
The rules are in `RULES` in `ui_start.rs`, plus four special cases. All
of them assume a new game's start in our client: in the main level, no
vehicle, no camera, the PDA closed, no messages.
- Made inactive in `Awake`: `uGUI_FeedbackCollector`'s `inputBlocker`,
  `root` and `repliesPanel`; `uGUI_PopupMessage`'s `root` (the critical
  warning, the HUD's message hint, the feedback hint and status);
  `uGUI_CameraCyclops`'s and `uGUI_CameraDrone`'s `content`.
- Made inactive in `Start`: `uGUI_SceneConfirmation` itself.
  `uGUI_BuildWatermark` hides itself once the main level is loaded and no
  wait screen is showing.
- Made inactive in the first `Update`: `uGUI_SeamothHUD`'s and
  `uGUI_ExosuitHUD`'s `root` (no vehicle) and `uGUI_RadiationWarning`'s
  `warning` (not radiated). Vehicles and radiation are not ported, so
  these are their start values.
- `CanvasGroup` alpha 0: `uGUI_Tooltip.Awake`,
  `uGUI_PopupNotification.Awake`, and `uGUI_MessageQueue` (the
  subtitles). For the queue, `OnEnable` sets `time = timeFadeOut`, so
  with no messages its first `Update` gives alpha 0.
- Graphics disabled: `uGUI_PowerIndicator.text` (no power outside a base
  or sub), and `uGUI_DepthCompass`'s four submersible graphics. In
  `DepthMode.Player`, `UpdateDepth` sets `submersibleDepth`, its suffix,
  `submersibleCrushDepth` and `submersibleBackground` to
  `enabled = false`.
- The compass: `uGUI_DepthCompass.LateUpdate` calls
  `compass.SetVisible(false)` when no compass is equipped. `uGUI_Compass`
  then builds an empty mesh (we draw it disabled) and disables its 8
  labels' texts.
- The intro (`uGUI_SceneIntro.Start`): `mainText.SetState(false)`
  disables the text on its own GameObject (`uGUI_Text.Awake` finds it
  with `GetComponent`). `skipText.SetAlpha(0)` sets TMP's `alpha`, which
  is its font colour's alpha. `skipProgress.value = 0` gives `_Value` 0,
  and the bar shader draws nothing at 0.
- `HandReticle`: `Awake` hides every icon (`uGUI_HandReticleIcon.SetActive
  (false, 0)`: its `go` inactive, its graphics' `CanvasRenderer` alpha 0
  on the first `UpdateIcon`). On the first `LateUpdate`, `desiredIconType`
  is still `None` (0), and the method ends by setting it to `Default`.
  On the second, the `Default` icon fades in over 0.1 s. Settled: 1 icon
  shown, 9 hidden. `Start` empties the four texts, and `UpdateText`
  makes an empty text's object inactive.

Unity runs `Awake` only once an object is active, so a rule whose
script's node is inactive at start is not applied, and that is logged.
None of these rules hit that case.

**Run** (2026-10-14, `sn-inspect ui --start`, 1920×1080): 37 start
notes. 66 draws remain, 23 of them visible, and 23 of 23 are named with
the rule that shows them:
- the four bars and their quad background (`uGUI_SceneHUD`, 12);
- the player depth (`DepthMode.Player`, 4);
- the reticle's `Default` icon (1);
- `DepthClear`, which our pass doesn't draw (1);
- the bars' pulse halos and wave (5): their pulse animation clips are not
  ported (M9u6b2), so these are still their stored state.
  Since M9u6b2, `ui --start` also runs the HUD's first frame with a new
  game's values. The clips then put the halos and the wave at alpha 0,
  and 21 of 21 visible draws are named.

The only draw larger than 5 % of its canvas is `DepthClear`.

**Found along the way, client:** the main world kept only the newest UI
build each frame. The worker sends each texture only once, so when two
builds arrived in one frame, the first one's textures were lost. With
fewer draws the worker got fast enough for this to happen at start, and
every image drew as a flat square (the resize log line was missing too).
Now each frame keeps every received build's textures and its resize.
After the fix, 72 textures are on the GPU, 62 of 62 draws are made, and
the pass costs 0.12 ms CPU per frame.

**The runs** (2026-10-13, Lifepod start, `--game-ui --shot 25`):
- 1920×1080 and 2560×1440 each give 96 draws, 1,024 vertices and 1,536
  indices. By shader: `UI/Default` 84, `UI/Simple` 10, `UI/Bar` 2 (drawn
  as `UI/Default`).
- Skipped: 77 texts, 1 `DepthClear`, and 1 draw without a placed canvas
  (the `Tooltip` canvas has no scaler).
- `ScreenCanvas` makes 96 draw calls with 54 textures; 69 textures are
  uploaded in all.
- The build takes about 1.05 s on the worker, after reading the install
  for 0.87–1.21 s. The item values take 0.07–0.09 ms each time the size
  changes.
- The pass costs 0.21 ms CPU per frame (mean of 600 frames). Every draw
  is made (96 of 96). There are no warnings, errors or panics in the
  logs.
- Screenshots: `out/m9u5c-1080.png` and `out/m9u5c-1440.png` (same
  layout, scaled).
- Without `--game-ui`, the client is unchanged (`out/m9u5c-off.png`).

### Text (M9u5d)

`game_ui_text.wgsl` ports `TextMeshPro/Distance Field` from vertex
program 5 and fragment program 13 (`UNDERLAY_ON` with
`UNITY_UI_CLIP_RECT`, the variant that covers every term). The other
variants are the same terms switched off by flags.
- **Vertex.** The vertex is offset by `_VertexOffsetX/Y`. The pixel size
  is `w ÷ (|P · screen| × _ScaleX/Y)`, and the SDF scale is
  `rsqrt(|pixel size|²) × |uv2.y| × _GradientScale × (_Sharpness + 1)`.
  With a perspective projection (`P[3][3] = 0`), the scale is mixed with
  `|scale| × (1 − _PerspectiveFilter)` by `|n · view|`, where `n` is
  `mul((0, 0, −1), (float3x3)WorldToObject)` normalized and `view` is the
  normalized direction from the vertex to the camera.
  - weight = `(lerp(_WeightNormal, _WeightBold, uv2.y ≤ 0) ÷ 4 +
    _FaceDilate) × _ScaleRatioA ÷ 2`;
  - bias = `0.5 − weight + 0.5 ÷ scale`;
  - underlay scale = `scale ÷ (1 + softness × scale)`, its bias =
    `(0.5 − weight) × layer scale − 0.5 − dilate × layer scale ÷ 2`
    (softness and dilate × `_ScaleRatioC`);
  - underlay UV = UV − offset × `_ScaleRatioC` × `_GradientScale` ÷ the
    atlas size;
  - `uv2.x` unpacks to the face/outline UV as `(⌊x ÷ 4096⌋, x mod 4096)
    ÷ 512`.
- **Fragment.** Face = vertex colour × `_FaceColor` × `_FaceTex`;
  outline = `_OutlineColor` × `_OutlineTex`; both premultiplied. With
  `d = bias − atlas.a` and outline width `w = _OutlineWidth × ratio A ×
  scale`, colour = `lerp(face, outline, sqrt(min(1, w)) × saturate(d ×
  scale + w ÷ 2))`. That is × `1 − saturate((d × scale − w ÷ 2 + s ÷ 2) ÷
  (1 + s))`, with `s` = softness × ratio A × scale. The underlay is
  `saturate(atlas.a(underlay UV) × layer scale − layer bias)` × its
  premultiplied colour, added × `(1 − alpha)`. Then the soft clip, then ×
  the vertex alpha. The output is premultiplied (`One OneMinusSrcAlpha`).
- **Our data.** `uv2.y` holds the scale up to the root canvas, and the
  shader multiplies it by the canvas's world scale (`Placement.scale.y`)
  to get TMP's `|lossyScale.y|`. The font atlases are R8 textures from
  `GlyphAtlases`, sent again whole when glyphs are added. `_FaceTex` and
  `_OutlineTex` are white: no in-scope material sets them, and the client
  logs any that does.

**The runs** (2026-10-13, Lifepod start, `--game-ui --shot 25`):
1920×1080 and 2560×1440 each give 171 of 171 draws, 3,720 vertices and
5,580 indices. By shader: TMP 75, `UI/Default` 84, `UI/Simple` 10,
`UI/Bar` 2 (drawn as `UI/Default`). 2 texts sit under the `Tooltip`
canvas, which has no scaler, and are not placed. Glyphs: `Aller_Rg SDF`
64 and `Aller_W_Bd SDF` 37, made in 0.69–0.74 s on the worker (slower
than `ui --draw`'s 0.33 s while the world loads beside it); no atlas
filled up. The pass costs 0.232 ms (1080) and 0.228 ms (1440) CPU per
frame, mean of 600, against M9u5's target of 0.5 ms. The item values
take 0.6–2.0 ms once per build. There are no warnings, errors or panics.
Screenshots: `out/m9u5d-1080.png` and `out/m9u5d-1440.png`: sharp text
in both faces, with the shadowed underlay style, and the same layout
scaled. The strings are the prefab's stored ones (placeholders and
untranslated keys); translation is `TranslateOnStart` and each screen's
rules (M9u6, M9u7).

## The live UI (M9u6a)

Our port: `crates/sn-assets/src/ui_scene.rs` (`UiScene`, `UiState`),
`crates/sn-ui/src/tween.rs`, the client's worker in
`apps/sn-client/src/game_ui.rs`. Check: `sn-inspect ui --live`.

A UI tree is read from the install once (its layout tree, canvases,
groups, masks, graphics and texts). After that, each draw takes a
`UiState`: the changes the game's scripts make through Unity's API.
Materials, font materials and sprites are read the first time a draw
needs them, then kept. Each change goes where uGUI puts it:

| Script call | Applied to |
|---|---|
| `GameObject.SetActive` | the node's `activeSelf` (layout and drawing) |
| `CanvasGroup.alpha` | the group's alpha; a node without a group gets none |
| `CanvasRenderer.SetAlpha` | the graphic's own alpha, × the inherited alpha |
| `Graphic.enabled` | drawn or not; a layout element or not |
| `Graphic.color` | an image's colour; a text's `m_fontColor` as `Color32` |
| `Image.sprite` | the mesh's sprite and texture, and the layout element's sprite size |
| `Image.fillAmount` | the fill, clamped to 0…1 (its setter clamps) |
| TMP `text` | the string, as set from code (`TextString`: escapes not parsed) |
| `anchoredPosition`, `localPosition.z`, `localRotation`, `localScale` | the RectTransform |

- **`Color` → `Color32`** — confirmed (decompiled
  `UnityEngine.CoreModule`): `(byte)Mathf.Round(Mathf.Clamp01(c) × 255)`.
  `Mathf.Round` is `Math.Round`, which rounds half to even.
- **The renderer's own alpha multiplies the inherited alpha** —
  *hypothesis* (the batching is native). `cullTransparentMesh` tests the
  product (*hypothesis*, as in § Draw order). A text's fallback
  sub-meshes take their text's alpha. In the game they are
  `TMP_SubMeshUI` children with their own renderers (*hypothesis*).
- **`Mathf.SmoothDamp`** — confirmed (decompiled `UnityEngine.
  CoreModule`): ω = 2 ÷ max(0.0001, smoothTime), x = ω dt, e = 1 ÷ (1 +
  x + 0.48 x² + 0.235 x³). The change is clamped to ± maxSpeed ×
  smoothTime, temp = (v + ω change) dt, v' = (v − ω temp) e, and the
  value = target + (change + temp) e. When that passes the original
  target, the value snaps to the target and v' = 0.
- **UWE's `Sequence`** — confirmed (decompiled `Assembly-CSharp`):
  `t` moves by ± dt ÷ time, clamped to 0…1. With time 0, `t` jumps to its
  end. The update after `t` reaches the target ends the sequence (that is
  when the callback runs). `ForceState` sets `t` to the state and leaves
  the sequence active, so the next update ends it.

**The runs** (2026-10-14, `ui --live`, uGUI at 1920×1080):
- Read into a scene in 0.011 s, after the install is indexed.
- The hashes are unchanged by the change. Stored state, unplaced:
  `83d22425b9e33ca7` (`ui --draw`'s uGUI). Placed: `f1365274ab2fbbf7`
  (`ui --canvas`). `ui --draw`'s 12 hashes and `ui --canvas`'s 4 equal
  the ones from before. Start state, placed: 175 draws,
  `95b81442602e32d6`.
- One change of each kind on a drawn HUD node (`SetActive`, text,
  `enabled`, `SetAlpha`, colour, scale, sprite) changes the draw list as
  expected. The start state drawn after each gives the start hash again.
  No HUD image is filled in the start state, so `fillAmount` is checked
  by unit tests only.
- 120 frames, each changing every HUD text (52), the alpha of every
  drawn HUD image (53) and the turn of the 8 bar icons: mean 2.66–2.81
  ms per draw, p95 2.4–3.1 ms (target written before the run: mean ≤ 4
  ms). Frame 0 took 42 ms, because it made 8 new digit glyphs (about 5
  ms each, M9u3's cost). The same state again took 2.6 ms.
- The first run's mean was 4.15 ms. 3 of the ~3.9 ms were texts, and
  half of that was copying, padding and interning their materials on
  every draw. Those are now kept per (material, font), which gives the
  same hash in less time.
- A fresh scene with fresh atlases draws the start state with the warm
  scene's hash.
- Client (`--game-ui --shot 25`, 1920×1080): 171 of 171 draws, as
  before. The first draw takes 830 ms, of which 814 ms is making 101
  glyphs. The pass costs 0.242 ms CPU per frame. 0 warnings or errors.

**Moved in M9u6b1.** `UiState` is now `sn_ui::state` (pure), so the
screens' rules can write it. Sprites are `SpriteId`s that the scene gives
out (`UiScene::sprite_id`). It also gained `material`: a node's
`MaterialValues` (floats, vectors and colours, keywords), which a script
writes into its graphic's own material instance. The scene copies the
graphic's material, applies the values and interns the result.
`mesh::color32` used to round half away from zero; it now rounds half to
even, as `Mathf.Round` does. No stored colour lands on an exact half:
the hashes are unchanged.

## The HUD's bars (M9u6b1)

Our port: `crates/sn-ui/src/bar.rs` (`CircularBar`),
`crates/sn-ui/src/hud.rs` (the rules), `crates/sn-assets/src/hud.rs`
(their fields and nodes), `apps/sn-client/src/game_ui_bar.wgsl`. Checks:
`sn-inspect ui --bars`; the client logs the bars each second.

**`uGUI_CircularBar`** — confirmed (decompiled `Assembly-CSharp`; fields
by `ui --layout`). This is a `Graphic` of the game's own. It builds no
mesh, so it draws `Graphic`'s quad (UVs 0…1, the colour as `Color32`).
Its layout sizes its rect to `_size` on both axes (M9u4a). It draws with
a material instance of `ShaderManager.preloadedShaders.UICircularBar`,
whose setters write:
- keywords `BORDER` and `ROUND_CORNERS` (from their fields), and
  `OVERLAY` when `_overlay` is set;
- `_Width` = (`_width` ÷ ppu) ÷ (`_size` ÷ ppu), where ppu is the
  canvas's `referencePixelsPerUnit` (100);
- `_EdgeWidth` and `_BorderWidth`, clamped to 0…1;
- `_BorderColor`, `_Overlay1_ST`, `_Overlay2_ST`, `_OverlayShift`,
  `_OverlayAlpha` (the setters clamp shift to −1…1 and alpha to 0…1);
- `_Value` = `Clamp01(value × _maxValue)`;
- `_MainTex` = `_texture`, or the white texture when it is null.

`value` is not serialized and starts at 0.75.

uGUI has 5 bars: the four HUD bars (size 115/52, width 34/20, border,
round corners) and the intro's `SkipProgress`. Only the oxygen bar sets
`_texture` and `_overlay`. The PDA screen has 1 more.

**`Hidden/UI/CircularBar`** — confirmed (UnityPy and `D3DDisassemble`,
throwaway scripts in the ignored `out/m9u6b/`). It is in
`preloadedshaders.asset_….bundle` (path id 4836221551473169727) and also
in `resources.assets`. One pass: `SrcAlpha OneMinusSrcAlpha`, no z
write, `unity_GUIZTestMode`, no culling, the stencil from `_Stencil…`.
There are 8 + 8 variants over `BORDER`, `OVERLAY` and `ROUND_CORNERS`;
each variant is the full program (15) with terms switched off.
- **Vertex.** The colour is vertex colour × `_Color`. The two overlay UVs
  are the UV through `_Overlay1_ST` and `_Overlay2_ST`.
- **Fragment.** With p = UV × 2 − 1 and w = `_Width`:
  - `_Value` ≤ 0 gives 0, and the pixel is not drawn.
  - v = 1 − |2 saturate((|p| + w − 1) ÷ w) − 1|: 1 in the middle of the
    ring, 0 at its edges.
  - v = 0 where the clockwise angle from the top, as a fraction of a
    turn, is past `_Value`. The program computes the angle from two
    `atan2` approximations: `atan2(x, y)` for x > 0, else `atan2(−x, −y)`
    + ½ turn.
  - `ROUND_CORNERS`: v = max(v, 1 − |p − c| × 2 ÷ w) for a cap at the
    start c = (0, 1 − w ÷ 2) and one at the end c = (sin a, cos a) × (1 −
    w ÷ 2), with a = `_Value` × 2π.
  - The hard clip on `_ClipRect`, then discard where v × clip < 0.001.
  - Colour = (`_MainTex` + `_TextureSampleAdd`) × colour. Where 0 < v <
    `_EdgeWidth`, alpha × smoothstep(v ÷ `_EdgeWidth`).
  - `BORDER`: where 0 < v < `_BorderWidth`, rgb = lerp(`_BorderColor`,
    rgb, smoothstep(v ÷ `_BorderWidth`)).
  - `OVERLAY`: `_OverlayTex` is sampled at each overlay UV, its x moved
    by −floor(y) × `_OverlayShift`. Each sample's rgb × a is added ×
    `_OverlayAlpha.x` (or `.y`).
- **Our port.** `_BorderColor` is a colour property and is made linear,
  like `_Color`. Vectors are not. *Hypothesis*: Unity converts colour
  properties in a linear project. Our WGSL samples both textures before
  any branch, because WGSL needs uniform control flow for derivatives;
  the result is the same.

**`uGUI_SceneHUD`** — confirmed (decompiled). `Awake` hides every
element. The game mode's options give its mode:
- `NoSurvival` with `NoOxygen` and `NoAggression`: 0, nothing shown;
- `NoSurvival` otherwise: 1, oxygen and health over `backgroundBarsDouble`;
- else: 2, all four bars over `backgroundBarsQuad`.

Its elements show while `IsActive`: the main level, not the intro, not
launching, a player not piloting and no cinematic, no camera drone. Our
`active` is "the player runs" (`Hud.shown`); the intro, piloting and the
drone are not ported. We play mode 2 (Survival).

**The bars** — confirmed (decompiled). Each bar's `LateUpdate` runs
only while its object is active, so a hidden bar keeps its value. In
`SetValue`, `curr` follows the target with `Mathf.SmoothDamp(curr,
target, vel, dampSpeed, ∞, PDA.deltaTime)`, starting at 0, and `bar.value
= curr`. `dampSpeed` is 0.1 for all four.
- Oxygen: the target is available ÷ capacity. The text is
  `RoundToInt(available)` of the input, not of `curr`. It is cached on
  (value, `RoundToInt(capacity)`).
- Health: the target is `Clamp01((health − tempDamage) ÷ maxHealth)`. The
  text is `CeilToInt(curr × maxHealth)`, every frame.
- Food and water: the target is value ÷ 100. The text is
  `CeilToInt(curr × 100)`, cached.
- The oxygen overlays: `overlayNST.w = −Time.time × overlayNSpeed`
  (0.25, 0.7). Each alpha = `Clamp01(EvaluateLine(0.5, 1, 1, 0,
  available ÷ capacity)) × overlayNAlpha` (0.75, 0.5), so they show from
  half oxygen down.
- `Mathf.RoundToInt` rounds half to even; `IntStringCache` gives the
  int's `ToString()`.
- `PDA.deltaTime` is `Time.deltaTime` unless the PDA pauses the game
  (not ported).

**Start values.** The main scene stores `Survival.food` and `water` as
0. `Survival.Start` calls `ResetStats`, which sets 50.5 and 90.5:
confirmed, read from the IL (`sn_dotnet::method_store_f32`). The bars
show these until eating and drinking are ported.

***Hypothesis*: float precision.** Our rules compute in single precision
op by op, as the decompiled C# reads. If the game's Mono runtime keeps
intermediates in double, results can differ in the last bit. That would
show where `CeilToInt` meets a whole number. For example, health after a
20-point hit settles at a `curr` just above 0.8, and our text shows "81"
(`ui --bars`, t = 31–35 s). Not compared with the game.

**The runs** (2026-10-14):
- `ui --bars` (a 75 s dive driven by `sn_sim::vitals`, drawn every frame
  at 1920×1080): 300 of 300 bar checks equal a replay written in the
  tool. Each check covers `_Value` in the drawn material, the text
  string, its quad count, and the oxygen ST and alpha. Draw mean
  2.6–2.9 ms, p95 2.6–3.3 ms. The first frame takes 300 ms (it makes
  glyphs).
- `ui --draw`: 12 of 12 hashes equal across two runs. 6 bar draws (5 in
  uGUI, 1 on the PDA screen).
- Client, `--game-ui`, starting at the Lost River (−950, −616, 1018):
  - the per-second log follows the simulation: oxygen 45 → 0 over 27 s,
    the bar's `_Value` lagging by the damping, its text the rounded
    oxygen;
  - 176 of 176 draws; the pass costs 0.24–0.33 ms CPU per frame;
  - the worker draws each frame in 3.3–4.4 ms (mean of 600, while the
    world streams);
  - 0 warnings or errors.
- Screenshot `out/m9u6b1-dive.png`: the bars are drawn bottom left, but
  the feedback panel, the confirmation dialog and the pop-ups (stored
  state, M9u6e and M9u7) cover them. The image is not a useful visual
  check yet.

## The bars' animations (M9u6b2) — confirmed (decompiled code, `ui --pulse`)

Ported in `sn-ui` (`clip`, `tween`, `hud`), read by `sn-assets::hud_config`,
checked by `sn-inspect ui --pulse`.

**The clips.** Each bar's root has a legacy `Animation` with one clip and
no default clip:
- `OxygenBarPulse` (1 s) on the oxygen bar: `PulseWave`'s scale (1.5 →
  4.5), and the `CanvasGroup` alphas of `PulseWave` and `PulseHalo`.
- `IconPulse` (1 s), the same clip on the health, food and water bars:
  `Icon`'s x and y scale (1 → 1.25 → 1, z stays 1, with stepped keys) and
  `PulseHalo`'s alpha.

All their curves are unweighted and clamped. Every alpha is 0 at both
ends, so a bar at rest shows no halo.

**Playing them.** In `Awake` each script stops the animation and sets its
only state to weight 1 and speed 0. In `OnEnable` it enables the state and
starts `pulseTween`, a looping `CoroutineTween`. Each frame:
- the tween's `OnPulse(scalar)` sets `normalizedTime = Clamp01((duration
  × scalar − pulseDelay) / pulseTime)`;
- the animation is sampled at that time;
- `LateUpdate` computes `pulseDelay` and `pulseTime` from
  `pulseDelayCurve` and `pulseTimeCurve` at `1 − Clamp01(value /
  pulseReferenceCapacity)`, each at least 0, and sets the tween's duration
  to their sum. When the sum drops to 0, `normalizedTime` resets to 0.

So a pulse is a delay with the clip at its start, then the clip played
over `pulseTime`, repeated. The stored `pulseReferenceCapacity` is 100 on
all four bars, including oxygen, whose code default is 45. With these
curves a bar pulses only once its value falls below 15 % of 100 (oxygen)
or 20 % (the others); the delay curve is −1, so 0, before that.

How Unity runs one frame (Unity's documented order):
1. `Update` (`uGUI_SceneHUD` shows the bars; `OnEnable` runs);
2. coroutines resume (`yield null`);
3. the legacy animation is sampled;
4. `LateUpdate`.

`StartCoroutine` runs an iterator to its first yield at once, so a tween
moves once when it starts.

- `CoroutineTween`: `MoveNext` adds the frame's delta to its time. The
  pulse uses `Time.deltaTime`; the punch uses `PDA.deltaTime`. In `Once`
  mode the time is clamped to the duration and the tween stops there. In
  `Loop` mode, `time %= duration`. With duration 0 it doesn't move. The
  `duration` setter keeps the time's fraction of the duration.
- Under `WrapMode.Loop` the state's time wraps at the clip's end, so a
  normalized time of 1 samples the start. This is a *hypothesis*; it
  shows nowhere, since every alpha is 0 at both ends.

**The icons** (health, food and water; `icon` is the bar's `Icon` node):
- **Punch:** `OnHealDamage`, `OnEat` and `OnDrink` call `Punch(2.5, 1 +
  Clamp01(amount / 100))`. `onHealDamage` fires only from
  `LiveMixin.AddHealth`, with the health gained; despite its name it is
  not fired by damage. The punch's tween calls `Oscillation(100, 5, seed,
  t)` with a random seed. `LateUpdate` adds `(o × max, o1 × max, 0)` to
  the icon's scale; the clip has just written that scale, so it doesn't
  build up. When the punch stops, the icon goes back to its scale from
  the punch's start.
- **Coin turn:** `LateUpdate` turns the icon about y towards 180° while
  the PDA is open (the numbers' side) and 0° otherwise, with
  `MathExtensions.CoinRotation`, a spring of 0.02 s steps. Each time the
  PDA opens or closes, the spin gets a random kick (`Random.Range(−v,
  v)`, v = `rotationRandomVelocity`).
- **`CoinRotation` as written:** it takes `FloorToInt(now − last)` steps,
  the whole seconds since `last`, not the 0.02 s steps in that time. So
  nothing moves in the first second after `OnEnable`. After that, `last`
  runs about a second behind and the spring takes one step per frame.
  Ported as written.

`UnityEngine.Random` is not reproduced: our `Hud` has its own generator,
used in the same ranges.

**Run** (2026-10-14, `sn-inspect ui --pulse`, 60 fps, 56 s). The script:
oxygen held at 45, then 6, 3 and 1 of 45; health 12 of 100 while oxygen
is at 1; a meal of 40 at 40 s; a heal of 30 at 42 s; the PDA open from 45
to 50 s. The replay is written in the run from the decompiled scripts,
with the curves read from `sn-unity`, not from our rules.
- 13,440 of 13,440 halo alphas in the draw list equal the replay's
  (within 1e-5).
- The pulse periods measured on the drawn halo are within one frame of
  the curves' delay + time:

  | Level | Delay + time | Measured mean |
  |---|---|---|
  | oxygen 6 | 0.64 s | 0.6398 s |
  | oxygen 3 | 0.52 s | 0.5205 s |
  | oxygen 1 | 0.44 s | 0.4404 s |
  | health 12 | 1.26 s | 1.2583 s |

- 300 of 300 icon widths during the punches equal the rest width × (the
  clip's scale + the punch), within 1e-3.
- With the PDA open, the health icon is within 2° of 180° after 0.8 s
  and settles on 180° by 1.3 s. Its drawn width follows |cos θ|: ×0.809
  at 216°.
- Client dive: the oxygen bar starts pulsing below 15 (period 2.48 s at
  12, 0.40 s at 0). 62 of 62 draws, 0 warnings. Screenshot
  `out/m9u6b2-dive.png`.

Our player is never healed and never eats or drinks yet, so the client
sends no punch events. The punches are checked only headless.

## The depth and compass (M9u6c research) — confirmed (decompiled code, IL, stored fields)

Read on 2026-10-14 from the decompiled `uGUI_DepthCompass`, `uGUI_Compass`,
`Ocean`, `MaterialExtensions` and `IntStringCache` (read only, in `out/`),
the IL (`sn-inspect code --il`) and the stored fields (`ui --layout`).
`uGUI_DepthCompass` is on `uGUI/ScreenCanvas/HUD/Content/DepthCompass`,
which `uGUI_SceneHUD` never deactivates, so its `LateUpdate` runs every
frame.

**`uGUI_DepthCompass.LateUpdate`** runs `Initialize`, `UpdateDepth`,
`UpdateCompass`, in that order.
- `Initialize` (once, when `Player.main` exists): the label keys
  `CompassDirection{label.name}`; `OnLanguageChanged` (`_meterSuffix` =
  `Language.Get("MeterSuffix")`, also into `submersibleDepthSuffix`; the
  compass labels' texts); `OnDepthClassChanged(player.depthClass)`.
- **`OnDepthClassChanged` ignores its argument.** The IL stores the value
  and then stores the constant 1 (`Safe`) over it (`ldc.i4.1; stloc.0`),
  as the decompiled `value2 = Ocean.DepthClass.Safe` shows. So the
  danger sprites, the danger colour and the alpha blend are never used:
  the shadow always gets `shadowNormal`, `SetBlending(Multiplicative,
  alphaPremultiply: true)` and both texts `textColorNormal` (stored
  (227, 243, 246, 255)). `_cachedDepthClass` starts at `Safe`, so the
  half moon is not re-picked here.
- `SetBlending(Multiplicative, true)` on the shadow's own material:
  `_SrcFactor` = `DstColor` (2), `_DstFactor` = `OneMinusSrcAlpha` (10),
  keyword `ALPHA_PREMULTIPLY` on (`SetBlendMode`).
- `UpdateDepth`: the mode is `None` when not initialized, not the main
  level, the intro, the rocket launching, a cinematic, or the PDA in use;
  `Submersible` in a vehicle; `MapRoomCamera` with a camera drone; `None`
  when piloting; else `Player` with depth `FloorToInt(Ocean.GetDepthOf
  (player))` = `floor(max(0, oceanLevel − y))`, the ocean level being
  `Ocean.main`'s y. On a mode change: `shadow`, `halfMoon`, `depthText`,
  `suffixText` enabled iff `Player` (then the half moon's sprite is
  picked); the four submersible graphics enabled for `Submersible` or
  `MapRoomCamera` (the crush depth for `Submersible` only). In `Player`,
  when the integer changes, `depthText` = `IntStringCache.GetStringForInt`
  (the integer's `ToString()`, cached) and `suffixText` = the suffix.
  Texts are not touched in `None`.
- `UpdateCompass`: enabled iff initialized, main level, no launch, no
  intro, the PDA not in use, not piloting, no cinematic, and the
  equipment holds a `Compass` (or a camera drone is in use). A change
  re-picks the half moon in `Player` mode. Then `compass.SetVisible
  (enabled)` and, when enabled, `compass.direction =
  MainCamera.camera.transform.eulerAngles.y / 360`.
- The half moon: `halfMoonCompassNormal` when the compass flag is 1, else
  `halfMoonNormal` (the danger pair is unreachable, above). The flag
  starts at −1, so before the first `UpdateCompass` the normal sprite is
  used.

**`uGUI_Compass`** (a `Graphic`; stored: `size` 3.9, `position` (0, −0.54,
−12.6), `rotation` (−70.8, 0, 0), `scale` 1, `scale2D` 342, `fov` 90,
`near` 0, `far` 10, `radius` 4.5, `alphaFrom` 0.375, `alphaTo` 0.625,
8 labels). `SetVisible(false)` empties its mesh and disables the labels'
texts. Its mesh is one quad of half-size `size`, moved by `TRS(position,
rotation, scale)`, projected by `Matrix4x4.Perspective(fov, 1, near,
far)` with `MultiplyPoint3x4` (no divide; with `near` 0 the third row
gives z = −(view z)), then x, y = `scale2D` × x / z; the UVs are stored
divided by z with 1/z in `uv1.x`, so the fragment's `uv / (1/z)` is
perspective-correct. `direction` (wrapped to 0…1, applied only when it
changes) sets `_Angle` = (`rotation.z` − 360 × direction) in radians,
`_AlphaFrom`, `_AlphaTo`, and each label's `anchoredPosition` (the point
at `radius` and angle `rotation.z + label.angle` through the same
matrix, rotated by the direction) and colour (white, alpha = sin(1 −
|2t − 1|), t = `Clamp01`((frac((rotation.z + angle − 90 − 360 ×
direction) / 360) − alphaFrom) / (alphaTo − alphaFrom))).
`uGUI/Compass`'s fragment (`out/m9u5/uGUI_Compass/`): p = 2 uv − 1; the
clockwise angle of p from +y as a fraction of a turn, through
`_AlphaFrom`…`_AlphaTo`, gives the same sin window as the labels; the
texture is sampled at p rotated by `_Angle`, (`_MainTex` +
`_TextureSampleAdd`) × colour, alpha × window × hard clip.

**Ported (M9u6c1)** in `sn_ui::depth`, read by `sn-assets::depth_config`,
checked by `sn-inspect ui --depth`. In our client the mode is `Player`
while the HUD is active (the player runs) and the PDA is closed, else
`None`; the depth is `max(0, ocean level − y)` of the simulated player's
position. `ui --depth` (2026-10-14): a 70 s timeline at 60 frames a
second (down to 139 m and back up, the PDA open twice, the HUD inactive
2 s, the compass flag on twice), drawn every frame: 91,230 of 91,230
checks equal the replay (player graphics drawn iff shown, the text the
last shown floor with one quad a digit in `textColorNormal`, the suffix
"m" from `MeterSuffix`, the half moon's sprite, the shadow's blend, the
labels drawn iff the compass shows, the submersible graphics and the
compass graphic never). Client dive (`--game-ui --start 0 -3 0 --look 20
-3 20 --look-down 60 --hold-forward 3 --panel 12=open,15=close`): in
29 of 29 shown seconds the text is `floor(depth)` (3 m to 19 m on the
sea floor); `None` with the text kept in the 3 seconds the PDA is open
and the 2 before the player runs; 0 warnings; `out/m9u6c1-dive.png`. Not
compared with the game on screen.

**Ported (M9u6c2)** in `sn_ui::compass` and `sn_ui::depth`, drawn as
`UiGraphicKind::Compass`, the shader as flag 16 of `game_ui.wgsl`,
checked by `sn-inspect ui --compass` (2026-10-14):
- **The labels' stored places are an oracle** (*confirmed*): the 8 stored
  `anchoredPosition`s equal `UpdateLabels`' at the stored `debugDirection`
  0.646 within 0.00005 px (ours, and an `f64` replay with explicit
  rotation matrices). This confirms the Euler order, the projection and
  that the labels' parent is the compass (the world round trip is the
  identity).
- 36 headings drawn as the client does: the ring's 4 vertices (worst
  1.7e-5), `_Angle`/`_AlphaFrom`/`_AlphaTo`, each label's place (worst
  5.3e-5 px) and its drawn alpha (exact, in bytes): 644 of 644 checks
  with the equip timeline (ring and labels drawn iff equipped and the PDA
  closed).
- The labels carry `uGUI_TextGradient`, an `IMeshModifier`; TMP's
  `TextMeshProUGUI` never calls mesh modifiers (none in its decompiled
  code), so it has no effect (*confirmed* by reading).
- The yaw is the main camera's `eulerAngles.y`: `atan2` of its forward's
  x and z, whatever the roll (Unity's decomposition).
- Client (`--game-ui --test-supply Compass --use-item 3 --panel
  6=take-all,7=close,8=open,9=equip,10=close,20=open,21=unequip,22=close
  --turn 30`): the compass shows from the equip, hides while the PDA is
  open and after the unequip; in 10 of 10 shown seconds the direction
  equals yaw ÷ 360; 71 of 71 draws, 0.14 ms CPU per frame, worker 3.2 ms;
  0 warnings. `out/m9u6c2-compass.png` shows the ring with SE and S at a
  heading near 160°. Not compared with the game on screen.

## TMP inline sprites (M9u6d1) — confirmed (decompiled TMP, IL, `ui --tmp-sprites`)

The hand reticle's texts end in the key to press, written as a TMP
`<sprite>` tag (M9u6d2). Read on 2026-10-14 from the decompiled
`TMP_Text` and `TextMeshProUGUI` (read only, in `out/`), `LanguageSDF`,
and the IL of `TMP_SpriteAsset` and `TMP_ExtensionMethods`. Our port is
`crates/sn-ui/src/text/sprite.rs` (with the tag in `input.rs`, the
layout in `generate.rs` / `preferred.rs`, the quad in `mesh.rs`); the
code is ours.

**The sprite assets.** Two are loaded: `InputSymbols` (164 sprites, all
unicode 0xFFFE, a 1024 × 512 `Alpha8` sheet, face point size 0, every
character and glyph scale 1.4, glyphs 48 × 48 with bearing (−8, 32.32)
and advance 32) and `EmojiOne` (16 sprites, a 512 × 512 sRGB sheet).
Stored `m_HashCode`s equal TMP's `GetSimpleHashCode` of the name
(`h = ((h << 5) + h) ^ c` over UTF-16 units) for all 180.

**The default sprite asset at run time is `InputSymbols`, not the stored
`EmojiOne`.** `LanguageSDF.PreloadFontAssets` loads its entry points
(`Aller_Rg SDF`, `Aller_W_Bd SDF`, `InputSymbols`), sets the first
sprite asset as `TMP_Settings.m_defaultSpriteAsset` (by reflection) and
adds any further ones as its fallbacks (none).

**The tag** (`ValidateHtmlTag`): `<sprite>` with an optional first
value (`=N`: an index; `="Asset"`: an asset by name; none: the text's
sprite asset, else the default) and attributes `name=` (found by its
hash in the asset, its fallbacks depth first, then the default asset),
`index=`, `tint=` (not 0: tinted), `color=` (hex) and `anim=`. Any other
attribute, an index out of range or a name not found rejects the tag,
and its characters are then laid out as text. `anim` is not ported (the
tag is rejected and counted). No in-scope text has its own sprite asset.

**The layout** (`GenerateTextMesh`, sprite branch): bold is cleared (and
stays cleared); a tag's character becomes U+E000 + its index, a sprite
found by its character gets white. For a sprite asset without a point
size (as `InputSymbols`) the scale is `font ascent line / glyph height
× character scale × glyph scale × (size / font point size × font face
scale)`, and the ascent and descent lines are the font's × (font scale ÷
that scale), so the sprite's height fits the font's ascent line. The
material padding is 0; the sprite is always visible. Kerning looks the
sprite's glyph index up in the font's pair table, as for a character.
At 24 px in `Aller_Rg SDF`, `MouseButtonLeft` is 44.0294 px square and
advances 29.3530 px (`ui --tmp-sprites`, equal to the formula in `f64`
within 1e-5).

**The quad** (`SaveSpriteVertexInfo`): UVs are the glyph rect in the
sheet, without padding. The colour is the tag's, × the text colour when
tinted (`Color32.Multiply`, IL: each channel `(byte)(a / 255f × b /
255f × 255f)` in `f32`, truncated). If its alpha is below the text
colour's it becomes `min(alpha, html colour alpha)`, else the text
colour's alpha. Sprites go into a sub-mesh per material.

**The material** — a Subnautica change to TMP: `ValidateHtmlTag` calls
`TMP_SpriteAsset.FindMatchingMaterial(m_currentMaterial)` (IL). Each
`m_materials` name's postfix is what follows its first space; the
longest postfix found in the text material's name (ordinal, ignoring
case) wins, the first on a tie; none found: the asset's `material`.
`InputSymbols` has `InputSymbols Atlas Material` and `m_materials`
`InputSymbols Outline` and `InputSymbols Shadow` (both `UNDERLAY_ON`),
**all `TextMeshPro/Distance Field`** sampling the sheet: the symbols are
a distance field, drawn with the text shader already ported (no
`TextMeshPro/Sprite` port needed). The shader reads the distance from
`.a`; our font atlases keep it in `.r`, so the client's text shader
reads `.a` for an install texture (text flag 4). *Hypothesis*: the sub-mesh's
material ratios and padding are set as a fallback material's
(`GetPadding`); the IL of `TMP_SubMeshUI` was not read.

**Not ported:** sprites found by their unicode (the default asset's are
all 0xFFFE), the text's own sprite asset, `anim`, sprite assets with
a point size are ported but none is in scope (untested on real data).

## The hand reticle (M9u6d2) — confirmed (decompiled code, IL, `ui --reticle`)

Read on 2026-10-14 from the decompiled `HandReticle`,
`uGUI_HandReticleIcon`, `Sequence`, `GUIHand`, `Pickupable`,
`BreakableResource`, `EscapePod`, `CinematicModeTrigger`, `GameInput`,
`GameInputSystem`, `AvatarInputHandler` and `Language` (read only, in
`out/`). Our port: `crates/sn-ui/src/reticle.rs` (rules),
`crates/sn-assets/src/reticle.rs` (config, `FormatButton`); the code is
ours.

**Stored** (`uGUI/ScreenCanvas/HUD/Content/HandReticle`): 10 icons under
`IconCanvas`, one per `IconType` 1–10 (`Progress` with 3 graphics, the
others 1); `iconScaleSpeed` 10; the icon canvas's scale 1; the four
texts `Hand`, `HandSubscript`, `Use`, `UseSubscript`.

**`LateUpdate`** (every frame): each text set and active iff not empty;
the icon is `None` while `hideCount > 0` or `!AvatarInputHandler.
IsEnabled()` (its cursor lock, off while the PDA is open); `SetIconInternal`
(on a change the old icon's `Sequence` goes to 0 and the new one's to 1
over 0.1 s, at once when the new one is `None`); the icon canvas's scale
lerps to `clamp(desired, 0, 2)` by `dt × 10`; the progress (on a change:
the image's fill and `HandReticleProgressPercentFormat`); every icon's
`UpdateIcon` (while its sequence runs: the game object active iff `t > 0`,
each graphic's `CanvasRenderer` alpha = `t`); then the reset: icon
`Default`, scale 1, progress 0, empty texts. So with nothing hovered the
`Default` dot shows.

**Start-state fix:** `uGUI_HandReticleIcon.go` is not stored (set in
`Awake` to its own game object). M9u6s looked it up as a field and so
left the 9 hidden icons active at alpha 0; they are now inactive, as
`Awake`'s `SetActive(false)` leaves them (`ui --live`'s start hash
changed from `7f52d63da4ad1e96` to `efa1606eb7c79aff`).

**What sets it** (`GUIHand.OnUpdate`, only while `IsEnabled()` and the
PDA is not in use, then the target's `OnHandHover`):
- pickup that fits: `PickUpFormat` (or `PackUpFormat` and icon `PackUp`
  when `usePackUpIcon`) with the left hand's button, icon `Hand`; the
  subscript is an `ISecondaryTooltip`'s (not ported);
- pickup that does not fit: the tech name, subscript `InventoryFull`, no
  icon (`Default`), no button; not pickupable: the name only;
- outcrop: its `breakText` with the button, icon `Hand`;
- the lifepod's locker: `EscapePodStorageOpen` with the button, icon `Hand`;
- a `CinematicModeTrigger` (the pod's ladder and hatches): its `handText`
  with the button, icon `Hand`, if `showIconOnHandHover` and no cinematic
  runs;
- the held item's `Use` texts (eat, drop, battery) are not driven (our
  player holds nothing).

**`GetText`**: the translated text, then `HandReticleAddButtonFormat`
(`{0} ({1})`) of it and `GameInput.FormatButton(button)`.
**`FormatButton(LeftHand)`**, keyboard, default bindings, every step
read from the code: `bindingsKeyboard` gives `<Mouse>/leftButton`; its
control's `displayName` in `Unity.InputSystem.dll`'s `MouseState` is
"Left Button" (*hypothesis*: what `ToHumanReadableString` with the device
omitted gives); `displayKeyboard` maps it to `MouseButtonLeft`; the
default sprite asset (`InputSymbols`) has that sprite, so the text is
`<sprite name="MouseButtonLeft" color=#ADF8FFFF>` (`defaultControlsColor`).
The player's own saved bindings are not read.

**Not ported or not reached:** `hideCount` (`RequestCrosshairHide`:
vehicles, cinematics), the screenshot hide, VR placement, the progress
(nothing calls `SetProgress` yet; `{0:P0}` formatted as `n%`,
*hypothesis* for Mono's English culture), `SetIcon` sizes other than 1,
and the game's Escape menu (our window's mouse grab is not the game's
cursor lock: the reticle hides only for the PDA).

**Checked:** `ui --reticle`, 22 of 22 checks and 6,480 frame checks
against a replay (0 differ); each fade 0.1000 s at 60 fps. Client: the
outcrop, the ore it drops, the ladder hatch and the locker each showed
the `Hand` icon and their text with the mouse icon; every fade done in
0.1 s plus less than one frame; `None` while the panel is open.
Screenshots `out/m9u6d2-outcrop.png`, `out/m9u6d2-locker.png`. Not
compared with the game on screen.

## Death, respawn and overlays (M9u6e1) — confirmed (decompiled code, IL, stored fields, `ui --death`)

Read on 2026-10-17 from the decompiled `uGUI_PlayerDeath`,
`PlayerAnimationEventManager`, `uGUI_SceneRespawning`, `uGUI_Fader`,
`uGUI_TextFade`, `uGUI_Text`, `uGUI_Overlays` and `Player` (`OnKill`,
`ResetPlayerOnDeath`, `SuffocationUpdate`, `SuffocationReset`), read
only, in `out/`. Our port: `crates/sn-ui/src/death.rs` (rules),
`crates/sn-assets/src/death.rs` (config); the code is ours.

**Stored** (`ui --death`, real-data test `the_death_screens_are_read`):
- `uGUI_PlayerDeath.blackOverlay`: `ScreenCanvas/PlayerDeathVignette/
  BlackOverlay`.
- `uGUI_SceneRespawning`: `loadingText` `Respawning/RespawningScreen/
  LoadingText`; `loadingBackground` a `uGUI_Fader` whose `canvasGroup` is
  on `Respawning`; `fadeInTime` 1, `fadeOutTime` 0.5, `useUnscaledTime`
  true, `initialState` false. `Loading` is "Loading..." in English.
- `uGUI_Overlays.overlays`: one, `OverlayCanvas/Overlays/OutOfOxygen`;
  its curve has keys (0, 0; slopes 3.148), (0.8, 1; 0), (1, 1; 0), clamped:
  0.60 at 0.25, 0.90 at 0.5, 1 from 0.8 on.

**The vignette** (`uGUI_PlayerDeath`; numbers read from the IL:
`TriggerDeathVignette`'s `Invoke`s, `Update`'s `ldc.r4; mul` and its
`Color.a` comparisons):
- `Start` sets the colour to (0, 0, 0, 0).
- `TriggerDeathVignette(type)` does nothing while `active`. `FadeToBlack`
  → `Invoke("BeginDeathFade", 0)`; `CutToBlack` → `CutToBlack()` at once
  (active, fading in, alpha 1, enabled); both `Invoke("BeginFadeOut", 6)`.
  `BeginDeathFade`: fading in, active, enabled. `BeginFadeOut`: not fading
  in.
- `Update`, while active: fading in, a colour with `a < 0.98` lerps to
  black by `Time.deltaTime × 5` (enabled each frame), else alpha 1;
  fading out, it lerps to clear by `deltaTime × 2`, and when the colour
  *before* the step is below 0.02, `ResetOverlay` (inactive, disabled,
  alpha 0).
- **Who triggers it:** the player animator's events, through
  `PlayerAnimationEventManager.DeathFade` / `DeathCut`
  (`SendMessage("TriggerDeathVignette", type)`). `Player.OnKill` sets the
  trigger `player_death` (fire and explosions pick others), whose state
  in layer 8 ("Death") plays `quick_death` (1 s, `DeathFade` at 0 s).
  The other death clips' events: `burn_death2` 1.814 s, `choke_death`
  1.687 s, `drown_death` 1.675 s (`DeathFade`), `explosion_death`
  1.151 s, `player_view_reaper_attack` 1.667 s (`DeathCut`).
  `cine_attack_kill`'s event is `deathCut` (lower case): no receiver has
  that method. In our client the event fired on the death state's first
  update (11 ms after the death, weight 0.105 inside the 0.1 s
  transition). *Hypothesis:* Unity also fires an event at time 0 on the
  state's first update.
- `EndSequenceManager` also triggers it (the ending; not ported).

**The respawn screen** (`uGUI_SceneRespawning`, `uGUI_Fader`):
- `Show`: `loadingText.SetText(Language.Get("Loading"))`, then
  `loadingBackground.SetState(true)` (`Sequence.ForceState`). `Hide`:
  `loadingBackground.FadeOut()`.
- `uGUI_Fader.FadeOut()` uses **`fadeInTime`** and `FadeIn()` uses
  `fadeOutTime` (the names are swapped in the code), so the respawn
  screen fades out over 1 s. `Update` (`ApplyState`), while the sequence
  is active: `sequence.Update(unscaledDeltaTime)` (this fader uses
  unscaled time), then `canvasGroup.alpha = 0.5 (1 − cos π t)`.
- `Player.ResetPlayerOnDeath` (a coroutine from `OnKill`): 5 s, then
  `respawning.Show()` just before `ToNormalMode` and the move to the
  respawn point; 1 s later and once the world is settled,
  `respawning.Hide()`, then `ResetHealth`, `oxygenMgr.Restore()` and
  `SuffocationReset()`. `UnstuckPlayer` also shows and hides it (not
  ported).

**The overlay** (`uGUI_Overlays`): `Set(id, value)` → the curve at
`value`, clamped to 0…1; above 0 the graphic is enabled and its
`CanvasRenderer` alpha set, else it is disabled (the alpha left as it
was). `Player.SuffocationUpdate` calls `Set(0, 1 − suffocation.t)` every
`Player.Update` (also while dead), and `OnKill` calls `Set(0, 1)`. After
a drowning the overlay stays at 1 until the respawn's
`SuffocationReset`; after a death with oxygen left (a fall) it shows for
the death's frame only (the next `Update` sets it back to 0).

**Frame order** (*hypothesis*, Unity's documented player loop, not read
from the engine): the scripts' `Update`s, then the delayed calls
(`Invoke`, and the coroutines' `WaitForSeconds`, so `Show` / `Hide`),
then the animators (their events). So a call changes what is drawn from
the next frame's `Update`, and `BeginDeathFade` (delay 0) runs on the
frame after the event, after that frame's `Update`.

**Draw order** (our port of M9u4d's rules): the respawn screen
(`ScreenCanvas/Respawning`) draws before the vignette
(`ScreenCanvas/PlayerDeathVignette`) and both before `OverlayCanvas`
(`ui --death`: items 45, 50 and 56 of 57). So after a drowning, with the
overlay and the vignette at 1, "Loading..." is covered; it shows only
through them as they fade. *Not compared with the game on screen.*

**Checked:** `ui --death`, 24 of 24 checks and 9,840 frame checks over a
scripted drowning against a replay (0 differ): black 0.783 s after the
event at 60 fps, fade out 6.000 s after it, the fader clear 1.000 s after
`Hide`; five frames drawn (the overlay, the black overlay and the respawn
screen's 2 draws visible exactly while their alphas are above 0); a
`CutToBlack` black on its frame; a death by damage shows the overlay on
one frame. Client drowning: see MODLOG (M9u6e1). Not compared with the
game on screen.

## Messages (M9u7 research) — confirmed (decompiled code, stored fields)

Read on 2026-10-17 from the decompiled `ErrorMessage`,
`uGUI_IconNotifier`, `uGUI_PopupNotification`, `Inventory`,
`Pickupable` and `Player` (read only, in `out/`).

- **They make objects at run time.** `ErrorMessage.GetEntry`
  instantiates `prefabMessage` four at a time under `messageCanvas`
  (`SetParent(messageCanvas, false)`, inactive) and pools them;
  `uGUI_IconNotifier.GetInstance` makes four `new GameObject
  ("NotifierIcon")` with a `uGUI_ItemIcon` whose `Init` builds its own
  children, and calls `SetAsLastSibling` on each icon it shows.
- **The templates are other roots of `uGUI.prefab`'s file** (`sn-inspect
  ui` lists 10): `prefabMessage` is `MessageInstance` (1 node, a TMP
  text); `NotificationLabel` (2 nodes) is the notification count's.
- **`ErrorMessage`** (on `uGUI`): `offset` (140, 140), `ySpacing` 10;
  `messageCanvas` is `ScreenCanvas/HUD/Content/ErrorMessageCanvas` (an
  override-sorting canvas). Times in the constructor: fly-in 0.3 s, delay
  5 s, fade-out 0.6 s, invisible 0.1 s. An equal text already shown is
  renewed and gets `" (x{n})"`. Every `AddError/Message/Warning/Debug` is
  the same call.
- **Who calls them in what we run:** `Player.ResetPlayerOnDeath` ends with
  `AddWarning(YouDied` or `YouDiedLostStuff)`; `Pickupable.OnHandClick`
  `AddWarning(InventoryFull)` when the pick-up fails;
  `Inventory.CanDropItemHere` `AddError(InventoryCantDropHere)`;
  `Inventory.Pickup` plays the icon (`uGUI_IconNotifier.Play(techType,
  From)`) and adds a notification (`NotificationManager.Add(Inventory,
  id, 4)`) unless `noMessage`.
- **`uGUI_IconNotifier`** (`ScreenCanvas/HUD/Content/IconNotifications`):
  `interval` 0.3 s, `iconSize` 64, two `IconAnimator`s (From, To) with
  curves for x, y, scale, alpha and an oscillation.
- **`uGUI_PopupNotification`**: the blueprint pop-up is `OnAnalyze`
  (`KnownTech`'s analysis unlocks); skins `Unlock` and `Journal` under
  `HUD/Content/PopupNotification`.

## Objects made at run time (M9u7a) — confirmed (`ui --instantiate`)

`UiScene::graft(template, parent)` is `Object.Instantiate(template)` then
`SetParent(parent, false)`: the template (a tree read alone, as any other
root of the file, with the same readers) is appended to the scene, its
root becoming the parent's last child with its stored local values and
the name `<name>(Clone)`. Appending keeps every stored index (the
screens' configs hold node indices); the draw order follows each node's
children list, not the node array, so the copy draws at its place in
the hierarchy. `set_as_last_sibling` moves a node to the end of its
parent's children. A subtree of `uGUI` read alone has the same
depth-first order as in the scene.

**Checked:** `ui --instantiate`, 10 of 10: a copy of `PlayerDepth` (6
nodes) beside it, given the originals' start state and moved by (37,
−23), draws 4 items equal to the original's 4 (node, kind, texture,
material, indices, colours, UVs) with every vertex moved by exactly the
offset, after the original's; four `MessageInstance` copies under
`ErrorMessageCanvas` draw in sibling order, and after
`SetAsLastSibling` on the first it draws last; the stored nodes' 55
start-state draws are unchanged.

## Text messages: `ErrorMessage` ported (M9u7b) — confirmed (`ui --messages`, client)

- **Read:** `messageCanvas` is `ScreenCanvas/HUD/Content/ErrorMessageCanvas`
  (at 1920×1080 its rect is (−960, −540, 1920, 1080)); `offset` (140, 140),
  `ySpacing` 10 (stored); `timeFlyIn` 0.3, `timeDelay` 5, `timeFadeOut`
  0.6, `timeInvisible` 0.1 (the constructor's initialisers, IL);
  `OnLateUpdate`'s `offsetY * 7f` and `offsetY < 1f` (IL). `prefabMessage`
  (`MessageInstance`): anchors and pivot at the top left (0, 1), size
  (500, 10), `m_fontColor` white; "Inventory full" measures 134.95 × 26.08
  (TMP's preferred size).
- **A message's life:** added at `PDA.time` with its end at time + 6 s;
  its entry flies from the canvas's centre (−½ min(500, preferred width),
  ½ preferred height) to (left + 140, top − 140 − the heights and
  spacings above) over 0.3 s with `EaseOutSine`; its alpha falls along
  `EaseOutSine` from 5.3 s to 5.9 s; it is removed on the first frame
  whose time is past its end, its height added to `offsetY`, which decays
  as `offsetY −= 7 · offsetY · dt` (0 below 1), so the ones below slide
  up. An equal text adds `" (x{n})"` and renews the end to time + 5.7 s
  (no flight).
- **The removal loop skips:** `messages.Remove` inside the `for` loop
  moves the next message into the index just checked, so two messages
  ending in the same frame go one frame apart (`ui --messages`: frames
  547 and 548).
- **`localPosition` from a `Vector2`** sets z to 0; the anchored position
  is `localPosition` less the anchor point (the canvas's top left for
  this template).
- **When in the frame** (`ManagedUpdate.Queue`'s order): `GUIHand`'s
  calls (`InventoryFull`) come before `PDA.UpdateTime`, coroutines'
  (`YouDied`) after the `Update`s (*hypothesis*: Unity's documented
  player loop), `OnLateUpdate` in `PreCanvasRectTransform`, before the
  canvases draw.
- **What our client reaches:** `Player.ResetPlayerOnDeath`'s last line,
  `YouDiedLostStuff` if `Inventory.LoseItems` dropped anything, else
  `YouDied`, once the controls are back; `Pickupable.OnHandClick`'s
  `InventoryFull` when the hand clicks a pickup with no room.
  `InventoryCantDropHere` comes only from `CanDropItemHere(notify: true)`,
  which only held tools and deployables call (not ported).

**Checked:** `ui --messages`, 7 of 7: 900 frames of a scripted timeline
(the game's strings; one message, the same text to "(x4)", two in one
frame, five at once, the pool growing from four to eight entries,
expiry while others stay, an entry reused) equal to a replay in doubles
(largest position error 0.0075 canvas units, f32 against double; alpha
0.000043); 90 draws: each shown entry draws one quad per visible
character at its alpha, the hidden ones nothing, the stored nodes as
before. Client drowning: "You died." at controls back, gone 6.0106 s
later (rule 6 s + under a frame).

## The unlock pop-up (M9u7d) — confirmed (decompiled code, stored fields, `ui --popup`)

Read on 2026-10-17 from the decompiled `uGUI_PopupNotification`,
`uGUI_PopupNotificationDefault`, `uGUI_PopupNotificationSkin`,
`KnownTech`, `Inventory`, `NotificationManager`, `MathExtensions`,
`RectTransformExtensions`, `ManagedUpdate` and the `Story` goals (read
only, in `out/`), and the stored fields (our layouts; throwaway probes in
the ignored `out/m9u7d/`). Our port: `crates/sn-ui/src/popup.rs`,
`crates/sn-assets/src/popup.rs`.

- **A pick-up never shows it.** `Inventory.Pickup` and `OnAddItem` call
  `KnownTech.Analyze(techType, verbose: false)`: the analysis's unlocks
  are added silently (no pop-up, no `Blueprints` notification).
  `OnAnalyze` acts only when `KnownTech.Add(techType)` is verbose: the
  added tech's own `analysisTech` entry is then analysed verbosely.
  Verbose adds come from `PDAScanner.Unlock` (scanning), `BlueprintHandTarget`
  (databoxes), `SpecimenAnalyzer`, `Rocket`, and story goals'
  `UnlockBlueprintData` with `Available` (e.g. `ItemGoalData`'s
  `OnCraftBattery` → `OnGoalUnlockData` unlocks the power cell;
  `OnCraftTank` the double tank; `OnCraftComputerChip` the advanced wiring
  kit; `OnConstructCyclops` the decoy and power upgrade modules). None of
  them is ported yet. `ItemGoalData` (58 goals) and `OnGoalUnlockData`
  (57 with a goal) are in the `main` scene bundle's shared file.
- **`NotificationManager` shows nothing on the HUD.** Its only views are
  `INotificationTarget`s (the PDA's item icons, blueprint, log and
  encyclopedia entries, pings) and listeners and counts (the PDA tabs:
  `uGUI_InventoryTab.notificationsCount` = `GetCount(Inventory)`).
  `LateUpdate` counts a notification down only while its target is
  visible; with a target registered but hidden, it is removed after more
  than 1 s of its duration has passed. `Inventory.Pickup` adds
  `(Inventory, the item's UniqueIdentifier id, 4 s)` unless `noMessage`;
  `KnownTech`'s verbose adds add `(Blueprints, key, 3 s)` and the
  `CraftTree`/`Builder` ones. Ported with the inventory tab (M9u8b).
- **`uGUI_PopupNotification`** (on `ScreenCanvas/HUD/Content/
  PopupNotification`: anchors y 0.4 stretched across, anchored (0, 108),
  size delta (0, 1080), so its rect is the canvas's width × 1080): its own
  `canvasGroup` (alpha 0 from `Awake`), `skins` `Unlock` and `Journal`
  (`uGUI_PopupNotificationDefault` each), `defaultDuration` 5 s,
  `ringDuration` 30 s (unused here). Sounds: `soundEncyUnlock` and others
  (not ported).
- **The skin** (`Unlock`: anchors and pivot (0, 0), 400 × 132; its own
  `canvasGroup`; `ox` = `oy` = 40): children `Background` (an `Image`,
  400 × 132), `Image` (256 × 128 at the left, the picture), `Text` (a TMP
  text at x 250 with a `ContentSizeFitter`, preferred width; colour #77CDFF),
  `Title` (above the top left; #B7DEFD) and `Controls` (below; empty for
  an analysis). `defaultSprite` is the picture when the entry has none;
  the skin's sounds are not set.
- **`Enqueue`** drops the entry's sound if a queued entry has the same
  one; **`Show`** sets the skin (`SetSkin`: each skin's group alpha 1 or
  0), `OnShow` (title, the image's sprite or `defaultSprite`, text,
  controls), `timeIn` 0.25, `timeDuration` = the entry's, `timeOut` 0.25,
  starts its sound (or the skin's `defaultSound`), `Zero` from `None`, and
  `start = PDA.time − timeIn × value` (so a shown pop-up does not jump).
  **`Hide`**: `start = PDA.time − (timeIn + max(0, duration) + timeOut ×
  (1 − value))`, the slide out from where it is. **`ShowNext`**: `Hide`
  unless a voiced sound plays, then first in the queue.
- **`OnUpdate`** (queue `UpdateAfterInput`, before `PDA.UpdateTime` in
  `UpdatePDA`): num = `PDA.time − start`; the phase is `None` until the
  first entry, else `Zero` (num < 0), `In` (< `timeIn`), `One` (< `timeIn`
  + duration, or duration < 0), `Out` (< + `timeOut`), else `Done`. On a
  change: the slide-in sound from `Zero`/`Done` to `In`/`One`, the
  slide-out sound from `In`/`One` to `Out`; the group's alpha = 1 from `In`
  to `Out`, else 0; `Out` stops the sound (fading); `Done` clears the
  entry. The value is `Trapezoid(0, timeIn, duration, timeOut, num)`
  (`Step` for a negative duration), kept unless not
  `Mathf.Approximately` the old one, which subscribes `OnLayoutComplete`
  (queue `UILayoutComplete`, later in the frame). At `None` or `Done` the
  next queued entry is shown. **`SetTransition(value)`**: the skin's
  anchored position = `Vector2.Lerp(p0, p1, EaseOutSine(value))` with
  `GetRectPositions(parent rect, skin rect, MiddleLeft, ox, oy)`: p0 =
  (−400, 474) beyond the left edge, p1 = (40, 474). An entry is up 0.25 +
  5 + 0.25 s; the next starts on the frame the last is done.
- **`OnAnalyze(analysis, verbose)`**: `Unlock` skin, `defaultDuration`,
  title `Language.Get(unlockMessage)`, text `Language.Get(techType.
  AsString())`, picture `unlockPopup`, sound `unlockSound`, then
  `Enqueue`. `PDAData.analysisTech`: 126 entries, 120 tech types (6 listed
  twice, the same; `Analyze` acts on the first), messages
  `NotificationBlueprintUnlocked` ("NEW BLUEPRINT SYNTHESIZED" in English)
  101, `NotificationCreatureDiscovered` 14, `NotificationBlueprintPickup` 5;
  60 with a picture; every one with the sound
  `event:/tools/scanner/new_blueprint`.
- **What our client reaches:** nothing of the game's; `sn-client
  --analyze <TechType>[,…]` is a test hook that calls `OnAnalyze` for
  each entry once the HUD is active.

**Checked:** `ui --popup`, 13 of 13: the config and skins read; four
`OnAnalyze` entries (power cell, decoy tube with its own picture, double
tank, and the advanced wiring kit by `ShowNext` at frame 450), titles and
texts from the language file; 1200 frames at 0.0173 s equal a replay
(value error 0.0000000, position ≤ 0.00004; the replay steps `PDA.time` as
a float, as the game does: summed in doubles it drifts by 1e-4 s over
19 s); shown in the order power cell, decoy, wiring kit, double tank, each
0.25 + 5 + 0.25 s after the last (the decoy hidden 0.25 s after
`ShowNext`); 240 draws: while shown, the background, picture, title and
text (the empty `Controls` has no vertices; the `Journal` skin at alpha
0), moving with the anchored position (≤ 0.0001); while hidden, none; the
other nodes' draws unchanged. Client (`--analyze PowerCell,CyclopsDecoyModule`): both shown in turn,
each done 5.5137 and 5.5010 s after it was shown (rule 5.5 s + under a
step), 0 warnings; a screenshot while the decoy's is shown. Not compared
against the game on screen.

## Picked-up icons (M9u7c) — confirmed (decompiled code, stored fields, `ui --icon-mesh`, `ui --icons`)

Read on 2026-10-17 from the decompiled `uGUI_IconNotifier`,
`uGUI_ItemIcon`, `uGUI_Icon`, `SpriteManager`, `RectTransformExtensions`,
`MathExtensions`, `ShaderManager` and `Inventory` (read only, in `out/`),
the stored fields and, for `UI/Icon`, UnityPy and `D3DDisassemble`
(throwaway scripts in the ignored `out/m9u7c/`). Our port:
`crates/sn-ui/src/icon.rs`, `crates/sn-ui/src/notifier.rs`,
`crates/sn-assets/src/icons.rs`.

- **`uGUI_IconNotifier`** (on `ScreenCanvas/HUD/Content/IconNotifications`,
  its `canvas`; at 1920×1080 that rect is (−960, −540, 1920, 1080)):
  `interval` 0.3 s, `iconSize` 64, two `IconAnimator`s, both 0.7 s,
  every curve clamped and unweighted. **From** (index 0, the pick-up's):
  x 0.5; y 0.5 → 0.475 at 0.4 → 0; scale 0.5 → 1 at 0.15 → 0.5; alpha
  0 → 1 at 0.1 → 1 at 0.6 → 0; oscillation from 0, reduction 130,
  frequency 5, × 4. **To** (batteries, crafting): y 0 → 0.5 at 0.5;
  scale 0.5 → 1 at 0.5 → 0.5; alpha 0 → 1 at 0.2 → 1 at 0.8 → 0;
  oscillation from 0.5, reduction 50, frequency 3, × 4. So a picked-up
  icon starts at the screen's centre (x and y are × the canvas rect's
  size from its bottom-left anchor) and sinks to the bottom edge.
- **The rules:** `Play` queues; `Update` takes one request when `PDA.time
  − time ≥ interval` (`time` starts at −interval), takes an icon from the
  pool (`GetInstance` makes four at a time), sets its sprite
  (`SpriteManager.Get(techType)`), `SetSize(64, 64)`, `SetAsLastSibling`,
  shows it, draws a `Random.value` seed. Each active request (last
  first) is animated at its t: anchored position (x, y) × the rect's
  size; scale = the curve + `Oscillation(reduction, frequency, seed,
  Clamp01(t − oscOffset) ÷ (1 − oscOffset) × duration)` × `oscScale`
  (from `oscOffset` on); `SetAlpha` = the alpha curve. On the frame t = 1
  it is released (hidden, back in the pool); otherwise t += `PDA.deltaTime`
  ÷ duration, clamped. With 0.3 s between icons and 0.7 s each, at most
  three are out at once: the first four icons are all the game makes.
- **The icon** is made by code: `new GameObject("NotifierIcon")` +
  `uGUI_ItemIcon` (a `Graphic` whose `OnPopulateMesh` clears: it draws
  nothing), `Init`'s `SetParams(anchor (0, 0), pivot (0.5, 0.5), canvas)`;
  its foreground (`CreateForeground`, on the first `SetForegroundSprite`)
  is a child `"Foreground"` with a `uGUI_Icon` (anchors and pivot at the
  centre, disabled until a sprite is set) and its own copy of
  `iconMaterial` (`GRAYSCALE` off, `_Chroma` 1, `NOTIFICATION` off,
  `_NotificationStrength` 0). `SetForegroundSize` fits the foreground to
  the sprite's rect keeping its aspect (`RectTransformExtensions.Fit`);
  `SetAlpha` sets the foreground's `CanvasRenderer` colour to white with
  that alpha.
- **`uGUI_Icon.OnPopulateMesh`** with a sprite (`GeneratePackedMesh`): the
  sprite's own mesh (`SpriteManager.GetSpriteCache`: `sprite.vertices`,
  `uv`, `triangles`), each vertex × (rect ÷ sprite size × pixels per
  unit), not moved by the rect's position (the sprite's pivot is the
  rect's origin); the colour is `Graphic.color` (white); the second UV is
  the vertex position. Without a sprite, one quad over the rect, UVs
  0…1. `GetPixelAdjustedRect` is the rect (no in-scope canvas is pixel
  perfect).
- **A sprite's mesh** is stored in `m_RD`: positions in stream 0 (float3,
  units about the pivot) and 16-bit indices. The atlas sprites' stored UV
  stream (stream 1) is all zeros. *Hypothesis* (Unity's native code): the
  engine derives `uv` from the render data's `uvTransform`: u = (x ×
  uvT.x + uvT.y) ÷ page width, v = (y × uvT.z + uvT.w) ÷ page height
  (uvT = pixels per unit and the pivot's place in the page per axis;
  titanium's is (100, 245, 100, 211) in a 1024×2048 page). `ui
  --icon-mesh` supports it: every UV of 21 sprites is within 0.15 px of
  its packed rect.
- **`UI/Icon`** (`ShaderManager.preloadedShaders.UIIcon`, from the
  Addressables key `Shaders/PreloadedShaders.asset`; in
  `preloadedshaders.asset_….bundle`, path id 7276442034518759825). One
  pass: blend `[_SrcFactor] [_DstFactor]` for colour and alpha (defaults
  5, 10: `SrcAlpha OneMinusSrcAlpha`), no z write, `unity_GUIZTestMode`,
  no culling, the stencil from `_Stencil…`. 128 + 128 variants over
  `ALPHA_PREMULTIPLY`, `FILL_RADIAL`/`_HORIZONTAL`/`_VERTICAL`,
  `GRAYSCALE`, `NOTIFICATION`, `SLICE_9_GRID`, `UNITY_UI_ALPHACLIP`. The
  variant with no keyword (the notifier's) is `UI/Simple`'s program
  without premultiplying: colour = vertex colour × `_Color`; (`_MainTex` +
  `_TextureSampleAdd`) × colour, alpha × the hard clip on `_ClipRect`.
  The others (fills, grayscale, the notification's god rays and overlay,
  slice 9) are not ported: a material with one of those keywords is drawn
  as `UI/Default` and named in the client's log. `iconMaterial` is `new
  Material(UIIcon)`: the shader's defaults (`sn_assets::new_material`).
  Its `_NotificationOverlayTex` (`Resources.Load("Sprites/
  NotificationOverlay")`) is not read: only the `NOTIFICATION` variants
  sample it.
- **When in the frame:** `Inventory.Pickup` (from `GUIHand`, in
  `ManagedUpdate`'s queue) calls `Play(techType, From)` unless
  `noMessage`; `uGUI_IconNotifier.Update` is a plain `Update`.
  `ManagedUpdate` and `uGUI_IconNotifier` both have execution order 0
  (their `MonoScript`s, read with UnityPy), so Unity's order between them
  is not defined. *Hypothesis:* the notifier runs after `PDA.UpdateTime`
  (with a steady frame rate, only which frame's `deltaTime` is used
  changes).
- **What our client reaches:** our own player's accepted pick-ups (the
  server's `PickedUp` event, so the icon starts when the event comes,
  a round trip after the click, not on the click's frame). `To` (battery
  swaps, crafting) and `Drillable`'s `From` (the exosuit's drill) are not
  reached.
- **`UnityEngine.Random`** is not reproduced: our generator (as M9u6b2).

**Checked:** `ui --icon-mesh`, 12 of 12: a `NotifierIcon` made by code
for each of 21 tech types (the last with no icon: `Unknown`) draws one
item, its foreground's, with `UI/Icon` and no keyword, the sprite's own
vertices (7 to 11) and triangles about one centre (to 0.00000), at most
30.5 from it (inside the 64 fit), every UV within 0.15 px of the packed
rect, the centres moving with the anchored positions; `SetAlpha(0.5)`
gives vertex alpha 128; the stored nodes' 55 draws unchanged. `ui
--icons`, 10 of 10: a scripted queue (three pick-ups in one frame, one
later, five more over three frames, one `To`) at 0.0173 s frames, 400
frames equal to a replay in doubles (position error ≤ 0.00017 canvas
units, scale ≤ 0.000002, alpha ≤ 0.000001); shown 0.3114 s apart (the
first frame past 0.3 s); at most three out, four icons made; 80 draws
equal (one item per shown icon at its alpha, scale and place, in the
order shown, none for pooled ones). Client (`sn-client --resolution 1920 1080 --start -197.3 -35.2 -10.2 --pick-up 3`): the three acid mushrooms picked up at 3 s give three `Play(AcidMushroom, From)` when the server's events come (one frame), shown 0.304 and 0.311 s apart; each released after its steps add up to 0.7092, 0.7107 and 0.7105 s (rule: at least 0.7 s, under one more step); 0 warnings; `out/m9u7c-pick.png` (3.5 s) shows the icon at the screen's centre.

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
