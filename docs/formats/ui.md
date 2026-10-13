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
