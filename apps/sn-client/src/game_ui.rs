//! The game's UI drawn as the game draws it (M9u5c; `docs/DESIGN.md` M9u
//! plan, `docs/formats/ui.md`). On by default since M9u6e2, which removed
//! our own HUD; `--no-game-ui` turns it off (then no HUD is drawn).
//!
//! - A worker thread reads the install once into a `sn_assets::UiScene`
//!   (M9u6a) and draws `uGUI`'s draw list from it with its canvases placed for the
//!   window's size (`sn_ui::scaler`: in front of the game's UI camera, sized
//!   from the main camera's field of view), again whenever the size
//!   changes. It sends the vertices, one draw per item, and each texture
//!   the first time it is used (decoded, bottom row first, so Unity's UVs
//!   sample it as they are).
//! - The main world turns textures into images and keeps the latest build.
//! - The render world draws it in a pass of our own after tonemapping (the
//!   game's UI camera draws after the main camera, clears nothing, and only
//!   `DepthClear` resets the depth, so the UI goes over the finished
//!   image), in the draw list's order (Bevy's transparent sort would
//!   reorder it), with each item's clip rect, softness and blend state.
//!
//! The HUD starts in its first-frame state (`sn_assets::Assets::
//! ui_start_state`: faders, overlays, the game-over blocker), not its
//! stored one, which is black.
//!
//! Shaders: `UI/Default`, `UI/Simple`, `UI/Icon`'s base, `SLICE_9_GRID`
//! and `NOTIFICATION` variants and `UI/Circle` (`game_ui.wgsl`, M9u8b2b,
//! M9u8b5b), and TMP's
//! `TextMeshPro/Distance Field` (`game_ui_text.wgsl`, M9u5d) with the
//! font atlases the worker fills (sent again when glyphs are added). `uGUI/DepthClear` writes only depth,
//! which our pass does not use: skipped (counted). Other shaders (`UI/Bar`,
//! `uGUI/WorldCursor`) are drawn as `UI/Default` and named in
//! the log until their steps port them.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::Core3d;
use bevy::core_pipeline::Core3dSystems;
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, storage_buffer_read_only, texture_2d};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BlendComponent,
    BlendFactor, BlendOperation, BlendState, Buffer, BufferInitDescriptor, BufferUsages,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, Extent3d, FragmentState, IndexFormat,
    LoadOp, Operations, PipelineCache, PrimitiveState, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipelineDescriptor, SamplerBindingType, ShaderStages, ShaderType,
    StorageBuffer, StoreOp, TextureDimension, TextureFormat, TextureSampleType, VertexAttribute,
    VertexFormat, VertexState, VertexStepMode,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::window::PrimaryWindow;
use sn_ui::death::{DeathInputs, DeathScreens, DeathType};
use sn_ui::depth::DepthCompass;
use sn_ui::hud::{Hud, HudInputs, hud_mode};
use sn_ui::inventory_tab::{InventoryTab, MakeIcon, UsedStorage};
use sn_ui::item_icons::{IconNodes, TabItem};
use sn_ui::messages::{MessageInputs, MessageLayout, Messages};
use sn_ui::notifier::{FROM, Icon, Notifier};
use sn_ui::pda::{PdaScreen, REGULAR_TABS};
use sn_ui::popup::{Entry, Phase, Popup, PopupLayout, TIME_IN, TIME_OUT};
use sn_ui::reticle::{HAND, HAND_SUBSCRIPT, HandReticle, IconType, ReticleInputs, get_text};
use sn_ui::scaler::{DEFAULT_UI_SCALE, Placement, UiCamera, place};
use sn_ui::state::UiState;
use sn_ui::toolbar::{Toolbar, ToolbarIcon, ToolbarSprites};

use crate::inventory::{ScreenClick, ScriptedHover};

mod drag;
mod notify;
mod tooltip;
use crate::textures::linear;

/// The main camera's planes in the main scene; the UI camera takes them
/// (`SNCameraRoot.Awake`; `docs/formats/ui.md` § Canvas placement).
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
/// Bytes per vertex: position (3 × f32), colour (4 × u8), uv, uv1.
const VERTEX_BYTES: usize = 32;

/// `UI/Icon`'s keywords (M9u7c1): a material with one other than
/// `SLICE_9_GRID` (M9u8b2b) and `NOTIFICATION` (M9u8b5b), whose variants
/// are ported, is drawn as `UI/Default` and named in the log.
const ICON_KEYWORDS: [&str; 7] = [
    "ALPHA_PREMULTIPLY",
    "FILL_HORIZONTAL",
    "FILL_RADIAL",
    "FILL_VERTICAL",
    "GRAYSCALE",
    "NOTIFICATION",
    "SLICE_9_GRID",
];

/// Item flags (`game_ui.wgsl`).
const SOFT_CLIP: u32 = 1;
const HARD_CLIP: u32 = 2;
const PREMULTIPLY: u32 = 4;
const MAIN_TEX_ST: u32 = 8;
const COMPASS: u32 = 16;
const SLICE_9: u32 = 32;
const CIRCLE: u32 = 64;
const NOTIFICATION: u32 = 128;

/// A clip rect that clips nothing (`UI/Simple` always tests `_ClipRect`;
/// *hypothesis*: Unity's native default covers everything).
const NO_CLIP: [f32; 4] = [-1e20, -1e20, 1e20, 1e20];

/// Unity's `BlendMode` values (`UnityEngine.Rendering.BlendMode`).
fn blend_factor(v: f32) -> BlendFactor {
    match v as i32 {
        0 => BlendFactor::Zero,
        1 => BlendFactor::One,
        2 => BlendFactor::Dst,
        3 => BlendFactor::Src,
        4 => BlendFactor::OneMinusDst,
        5 => BlendFactor::SrcAlpha,
        6 => BlendFactor::OneMinusSrc,
        7 => BlendFactor::DstAlpha,
        8 => BlendFactor::OneMinusDstAlpha,
        9 => BlendFactor::SrcAlphaSaturated,
        _ => BlendFactor::OneMinusSrcAlpha,
    }
}

/// Source and destination factors, colour then alpha (Unity's values).
type BlendKey = [u8; 4];
const ALPHA_BLEND: BlendKey = [5, 10, 5, 10];

/// One draw: a range of the index buffer with its texture and values.
#[derive(Clone, Debug)]
pub(crate) struct UiDrawCmd {
    pub first_index: u32,
    pub index_count: u32,
    /// The texture's key (0: white).
    pub texture: u64,
    /// The root canvas whose placement maps it.
    pub root: usize,
    pub blend: BlendKey,
    /// `_Color` (linear), `_MainTex_ST`, `_ClipRect`, softness.
    pub color: [f32; 4],
    pub st: [f32; 4],
    pub clip: [f32; 4],
    pub softness: [f32; 2],
    pub flags: u32,
    /// `UI/Icon`'s `NOTIFICATION` (M9u8b5b): `_NotificationStrength`,
    /// `_PDATime`, `_Size`; and `_NotificationOverlayTex`'s key (0: white).
    pub notification: [f32; 4],
    pub overlay: u64,
    /// A TMP text's material values (`game_ui_text.wgsl`); `None` for an
    /// image.
    pub text: Option<TextValues>,
    /// A `uGUI_CircularBar`'s material values (`game_ui_bar.wgsl`, M9u6b1).
    pub bar: Option<BarValues>,
}

/// What a draw is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Kind {
    Image,
    Text,
    Bar,
}

impl UiDrawCmd {
    fn kind(&self) -> Kind {
        if self.text.is_some() {
            Kind::Text
        } else if self.bar.is_some() {
            Kind::Bar
        } else {
            Kind::Image
        }
    }
}

/// A `Hidden/UI/CircularBar` material's values (colours linear).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BarValues {
    pub overlay1_st: [f32; 4],
    pub overlay2_st: [f32; 4],
    /// `_Width`, `_Value`, `_EdgeWidth`, `_BorderWidth`.
    pub bar: [f32; 4],
    pub border_color: [f32; 4],
    /// `_OverlayShift.xy`, `_OverlayAlpha.xy`.
    pub overlay: [f32; 4],
    /// 1 `BORDER`, 2 `OVERLAY`, 4 `ROUND_CORNERS`.
    pub flags: u32,
    /// `_OverlayTex`'s key (0: none).
    pub overlay_texture: u64,
}

/// A `TextMeshPro/Distance Field` material's values (colours linear).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TextValues {
    pub face_color: [f32; 4],
    pub outline_color: [f32; 4],
    pub underlay_color: [f32; 4],
    /// `_FaceDilate`, `_OutlineWidth`, `_OutlineSoftness`, `_ScaleRatioA`.
    pub face: [f32; 4],
    /// `_WeightNormal`, `_WeightBold`, `_GradientScale`, `_Sharpness`.
    pub weight: [f32; 4],
    /// `_ScaleX`, `_ScaleY`, `_PerspectiveFilter`.
    pub scale: [f32; 3],
    /// `_UnderlayOffsetX/Y`, `_UnderlayDilate`, `_UnderlaySoftness`.
    pub underlay: [f32; 4],
    /// `_ScaleRatioC`, `_TextureWidth`, `_TextureHeight`.
    pub atlas: [f32; 3],
    /// `_VertexOffsetX/Y`, `_MaskSoftnessX/Y`.
    pub offset: [f32; 4],
}

/// Text item flags (`game_ui_text.wgsl`).
const TEXT_CLIP: u32 = 1;
const TEXT_UNDERLAY: u32 = 2;
/// The atlas is a texture of the install (a sprite asset's sheet, M9u6d1):
/// its distance is in `.a`, as Unity's shader reads it; our font atlases
/// keep it in `.r`.
const TEXT_ALPHA_ATLAS: u32 = 4;
/// TMP's blend: premultiplied (`One OneMinusSrcAlpha`).
const PREMULTIPLIED_BLEND: BlendKey = [1, 10, 1, 10];

/// How a texture's bytes are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TexelFormat {
    Rgba,
    RgbaSrgb,
    /// One byte per texel (a font atlas's distance field; TMP samples its
    /// `Alpha8` texture's alpha, our shader the red channel).
    R8,
}

/// A texture as the GPU takes it.
pub(crate) struct UiTexture {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    /// Bottom row first.
    pub data: Vec<u8>,
    pub format: TexelFormat,
    pub repeat: bool,
    pub point: bool,
    /// Alpha only (`_TextureSampleAdd` (1, 1, 1, 0)).
    pub alpha_only: bool,
}

/// What the worker sends for one window size.
pub(crate) struct UiBuild {
    pub screen: [i32; 2],
    pub field_of_view: f32,
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub draws: Vec<UiDrawCmd>,
    pub placements: HashMap<usize, Placement>,
    /// The PDA screen's canvas node (M9u8a4a): its pose comes from
    /// [`GameUiFrame::pda_pose`], not from `placements`.
    pub pda_root: Option<usize>,
    /// Textures not sent before.
    pub textures: Vec<UiTexture>,
    pub seconds: f64,
    /// Built for a new size (logged in full); a frame's build is not.
    pub resized: bool,
    /// Item clicks found since the last build (M9u8b3a).
    pub item_clicks: Vec<(u64, u8)>,
    pub item_moves: Vec<sn_protocol::Request>,
    /// The tooltip while shown (M9u8b3b3): placed from the PDA's pose each
    /// frame.
    pub tooltip: Option<TooltipOnPda>,
}

/// Where the tooltip is on the PDA screen's canvas (`UpdatePosition`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct TooltipOnPda {
    /// `uGUI/Tooltip`'s node.
    pub root: usize,
    /// Its pivot (top left) in the PDA canvas's space.
    pub point: [f32; 3],
    /// Its scale ÷ the PDA canvas's.
    pub k: f32,
}

/// What the main world sends the worker.
enum Request {
    /// The window's size.
    Size([i32; 2]),
    /// A frame's inputs to the HUD's rules (M9u6b1), the hand's hover
    /// (M9u6d2), the death screens' calls (M9u6e1), `ErrorMessage`'s
    /// (M9u7b: language keys, before and after `PDA.UpdateTime`) and
    /// `uGUI_IconNotifier.Play`'s (M9u7c2: the tech types' names).
    /// Boxed: the other variant is small.
    Frame(Box<FrameInputs>),
}

/// [`Request::Frame`]'s inputs.
type FrameInputs = (
    HudInputs,
    HandFrame,
    DeathInputs,
    MessageKeys,
    Vec<String>,
    PdaFlags,
    PdaClicks,
    TabContents,
);

/// One item for the inventory tab's icons (M9u8b2a): our id, tech type,
/// place and size in cells, and the player's slots that take it
/// (`Equipment.GetSlots(TechData.GetEquipmentType)`, M9u8b3a's highlight).
#[derive(Clone, Debug, PartialEq)]
struct TabEntry {
    id: u64,
    tech: i32,
    at: (i32, i32),
    size: (i32, i32),
    compatible: Vec<String>,
    /// What the tooltip reads (M9u8b3b3): `GetItemAction(item, 0)` and
    /// `(item, 1)` (`sn_sim::inventory::item_action`), `GetCanBindItem`
    /// (a `Hand` item in the player's grid), and the item's prefab class.
    actions: [u32; 2],
    bind: bool,
    class: Option<String>,
    charge: Option<f32>,
}

/// Our player's containers this frame, for the inventory tab's icons.
#[derive(Clone, Debug, Default, PartialEq)]
struct TabContents {
    stores: Option<drag::Stores>,
    inventory: Vec<TabEntry>,
    locker: Vec<TabEntry>,
    /// (slot, item) of the player's equipped items.
    equipment: Vec<(String, TabEntry)>,
}

/// This frame's clicks for the PDA's toolbar (M9u8a4b) and the screen's
/// pose they are made on (the last frame's [`GameUiFrame::pda_pose`]).
#[derive(Clone, Debug, Default)]
struct PdaClicks {
    clicks: Vec<ScreenClick>,
    pose: Option<([f32; 3], [f32; 4])>,
    /// The pointer while the screen takes clicks (M9u8b3a), physical
    /// pixels from the bottom left.
    pointer: Option<[f32; 2]>,
    /// `--panel`'s scripted pointer (M9u8b3b3): over it, not the mouse.
    hover: Option<ScriptedHover>,
    drag: Vec<crate::inventory::DragInput>,
}

/// The PDA's flags this frame (M9u8a4a, `sn_sim::pda::Pda`): their changes
/// are `uGUI_PDA`'s calls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PdaFlags {
    /// `using_pda`: up at `Open` (`OnOpenPDA`), down at `Close`
    /// (`OnClosePDA`).
    using: bool,
    /// Focused: up at `Activated` (`OnPDAOpened`).
    focused: bool,
    /// The model active: down at `Deactivated` (`OnPDAClosed`).
    active: bool,
    /// Opened with the lifepod's locker as the used storage (M9u8b1).
    with_locker: bool,
}

/// This frame's `ErrorMessage` calls (M9u7b), as language keys.
#[derive(Clone, Debug, Default)]
struct MessageKeys {
    early: Vec<&'static str>,
    late: Vec<&'static str>,
}

/// What the hand reticle's rules read (M9u6d2): this frame's
/// `OnHandHover` calls.
#[derive(Clone, Debug, Default)]
struct HandFrame {
    hover: Option<crate::player::HandHover>,
}

/// The worker's ends of the channels.
#[derive(Resource)]
struct UiWorker {
    requests: Sender<Request>,
    builds: Mutex<Receiver<Result<UiBuild, String>>>,
    requested: Option<[i32; 2]>,
}

/// The latest build and the textures, for the render world.
#[derive(Resource, Clone, Default, ExtractResource)]
pub(crate) struct GameUiFrame {
    build: Option<Arc<UiBuild>>,
    generation: u64,
    /// The PDA screen canvas's pose in the UI camera's space (Unity's
    /// axes): `ScreenAnchor` taken into the main camera's space this frame
    /// (`uGUI_CanvasScaler`'s Inversed mode, M9u8a4a). `None` while the PDA
    /// is not active.
    pda_pose: Option<([f32; 3], [f32; 4])>,
    images: Arc<HashMap<u64, (Handle<Image>, bool)>>,
    white: Handle<Image>,
    /// Item clicks the worker found on the PDA's inventory tab (M9u8b3a):
    /// (item, button), for `crate::inventory` to act on.
    pub(crate) item_clicks: Vec<(u64, u8)>,
    pub(crate) item_moves: Vec<sn_protocol::Request>,
}

/// Marks the camera the UI is drawn over.
#[derive(Component, Clone, Copy, ExtractComponent)]
pub(crate) struct GameUiCamera;

pub(crate) struct GameUiPlugin {
    pub game_dir: Option<PathBuf>,
    /// The main camera's field of view (degrees), which the UI camera
    /// takes.
    pub field_of_view: f32,
    /// `--analyze` (M9u7d2): tech types whose unlock pop-up is shown once
    /// the HUD is active.
    pub analyze: Vec<String>,
}

impl Plugin for GameUiPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "game_ui.wgsl");
        bevy::asset::embedded_asset!(app, "game_ui_text.wgsl");
        bevy::asset::embedded_asset!(app, "game_ui_bar.wgsl");
        let (requests, worker_requests) = channel::<Request>();
        let (worker_builds, builds) = channel();
        let game_dir = self.game_dir.clone();
        let fov = self.field_of_view;
        let analyze = self.analyze.clone();
        std::thread::Builder::new()
            .name("game-ui".into())
            .spawn(move || worker(game_dir, fov, analyze, worker_requests, worker_builds))
            .expect("spawn the game UI worker");
        app.insert_resource(UiWorker {
            requests,
            builds: Mutex::new(builds),
            requested: None,
        })
        .init_resource::<GameUiFrame>()
        // The player's feed; a fly-camera run has no player, and the HUD
        // stays inactive.
        .init_resource::<crate::player::Hud>()
        .add_plugins((
            ExtractResourcePlugin::<GameUiFrame>::default(),
            ExtractComponentPlugin::<GameUiCamera>::default(),
        ))
        .add_systems(Startup, make_white)
        .add_systems(PostStartup, mark_camera)
        // After every `Update` system: the player, the animators (their
        // death events) and the camera have run for this frame.
        .add_systems(
            PostUpdate,
            (request_build, send_inputs, receive_build).chain(),
        )
        // The PDA screen's pose once this frame's transforms are final.
        .add_systems(
            PostUpdate,
            pda_pose
                .after(receive_build)
                .after(bevy::transform::TransformSystems::Propagate),
        );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<GpuUi>()
            .add_systems(RenderStartup, init_gpu)
            .add_systems(
                Render,
                (
                    prepare_pipelines.in_set(RenderSystems::Prepare),
                    prepare_buffers.in_set(RenderSystems::PrepareResources),
                ),
            )
            .add_systems(
                Core3d,
                game_ui_pass
                    .in_set(Core3dSystems::PostProcess)
                    .after(tonemapping),
            );
    }
}

fn mark_camera(mut commands: Commands, cameras: Query<Entity, With<Camera3d>>) {
    for e in &cameras {
        commands.entity(e).insert(GameUiCamera);
    }
}

fn make_white(mut images: ResMut<Assets<Image>>, mut frame: ResMut<GameUiFrame>) {
    frame.white = images.add(Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![255; 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));
}

// ---- The worker ----

fn worker(
    game_dir: Option<PathBuf>,
    fov: f32,
    analyze: Vec<String>,
    requests: Receiver<Request>,
    builds: Sender<Result<UiBuild, String>>,
) {
    if let Err(e) = run_worker(game_dir, fov, analyze, &requests, &builds) {
        let _ = builds.send(Err(e));
    }
}

fn run_worker(
    game_dir: Option<PathBuf>,
    fov: f32,
    analyze: Vec<String>,
    requests: &Receiver<Request>,
    builds: &Sender<Result<UiBuild, String>>,
) -> Result<(), String> {
    let start = Instant::now();
    let game = sn_install::GameData::locate(game_dir).map_err(String::from)?;
    let assets = sn_assets::Assets::index(&game)?;
    let tags = sn_assets::physics_settings(&assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(&game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let mut atlases = sn_assets::GlyphAtlases::new(&fonts)?;
    let catalog = assets.catalog()?;
    let root = assets.ugui_root(&catalog)?;
    let tree = assets.ui_tree("uGUI.prefab", &root)?;
    let scalers = assets.ui_scalers(&tree)?;
    // The scripts' first-frame changes: the stored state has full-screen
    // black faders and overlays the game hides at start.
    let start_state = assets.ui_start_state(&layouts, &tree)?;
    for n in &start_state.notes {
        info!("game UI start: {n}");
    }
    let start_state = sn_assets::UiState::from(&start_state);
    // Read once; each build only draws it (M9u6a).
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    // The HUD's rules (M9u6b1): `Survival`'s start food and water are in
    // its code.
    let mut hud = Hud::new(assets.hud_config(&layouts, &tree)?);
    // The depth readout (M9u6c1): its strings from the language file
    // (English, the only language read for now).
    let language = sn_assets::language(&game, "English")?;
    let mut depth = DepthCompass::new(assets.depth_config(&layouts, &tree, &mut scene, &language)?);
    // The hand reticle (M9u6d2) and the left hand's button text.
    let mut reticle = HandReticle::new(assets.reticle_config(&layouts, &tree, &language)?);
    let left_hand = sn_assets::format_button(&game, &set, &language, "LeftHand")?;
    info!(
        "game UI reticle: FormatButton(LeftHand): binding {:?} → {:?} → {:?} → {:?}",
        left_hand.binding, left_hand.readable, left_hand.id, left_hand.text
    );
    // Death, respawn and the overlays (M9u6e1).
    let code = sn_assets::read_assembly(&game)?;
    let mut death = DeathScreens::new(assets.death_config(&layouts, &tree, &code, &language)?);
    {
        let c = &death.config;
        let v = c.vignette;
        info!(
            "game UI death: vignette Invoke {} s / {} s, rates {} / {}, limits {} / {}; respawn fader fade-in {} s, fade-out {} s (FadeOut uses fade-in), {:?}; {} overlays",
            v.begin_fade_delay,
            v.fade_out_delay,
            v.rate_in,
            v.rate_out,
            v.full,
            v.clear,
            c.loading_background.fade_in_time,
            c.loading_background.fade_out_time,
            c.loading,
            c.overlays.len()
        );
    }
    // The HUD's text messages (M9u7b): `ErrorMessage` and its
    // `prefabMessage`, grafted four at a time as it asks.
    let (message_config, message_template) =
        assets.messages_config(&layouts, &tree, &code, &fonts, &tags.sorting_layers)?;
    {
        let t = message_config.times;
        info!(
            "game UI messages: canvas {}, offset {:?}, ySpacing {}; fly-in {} s, delay {} s, fade-out {} s, invisible {} s; slide × {}, snap {}",
            tree.nodes[message_config.canvas].path,
            message_config.offset,
            message_config.y_spacing,
            t.fly_in,
            t.delay,
            t.fade_out,
            t.invisible,
            t.slide_rate,
            t.snap
        );
    }
    let mut messages = Messages::new(message_config);
    let scale_modes: HashMap<usize, i32> =
        scalers.iter().map(|(n, s)| (*n, s.scale_mode)).collect();
    // Read from the laid-out scene when first needed after a resize.
    let mut message_layout: Option<MessageLayout> = None;
    // Each shown message's entry → (text, first shown, last renewed).
    let mut message_log: HashMap<usize, (String, f32, f32)> = HashMap::new();
    // The picked-up icons (M9u7c2): `uGUI_IconNotifier`, its icons made by
    // code (`NotifierIcon` + its foreground) with `iconMaterial`, and the
    // item sprites `SpriteManager` looks up.
    let mut notifier = Notifier::new(assets.notifier_config(&layouts, &tree)?, 0x5eed_1c0f);
    let mut icon_material = assets.icon_material(&catalog, &layouts)?;
    // `iconMaterial`'s `_NotificationOverlayTex` (M9u8b5b): `Resources`
    // `Sprites/NotificationOverlay`, sent with every build's textures.
    let overlay = assets.resource_texture("Sprites/NotificationOverlay")?;
    match &overlay {
        Some(o) => {
            icon_material
                .textures
                .insert("_NotificationOverlayTex".into(), sn_assets::texture_key(o));
        }
        None => warn!("game UI: Resources Sprites/NotificationOverlay not found"),
    }
    let icon_template = sn_assets::UiScene::from_code(
        "NotifierIcon",
        sn_assets::notifier_icon_nodes(tree.nodes[notifier.config.canvas].layer, &icon_material),
    )?;
    let item_sprites = assets.sprites(&catalog)?;
    {
        let c = &notifier.config;
        info!(
            "game UI icons: canvas {}, interval {} s, iconSize {}; {} animators (durations {:?}); iconMaterial {} with keywords {:?}",
            tree.nodes[c.canvas].path,
            c.interval,
            c.icon_size,
            c.animators.len(),
            c.animators.iter().map(|a| a.duration).collect::<Vec<_>>(),
            icon_material.shader,
            icon_material.keywords
        );
    }
    // `canvas.rect`, read when first needed after a resize.
    let mut notifier_rect: Option<sn_ui::layout::Rect> = None;
    // Each shown icon's root: (sprite name, shown at).
    let mut icon_log: HashMap<usize, (String, f32, f32)> = HashMap::new();
    // The previous busy frame's step, for the release check.
    let mut icon_last_step = 0.0f32;
    // Each queued request's sprite name, in queue order.
    let mut icon_names: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    // The unlock pop-up (M9u7d1–d2): `uGUI_PopupNotification` and its
    // skins. Its only input is `--analyze`: `OnAnalyze`'s sources
    // (crafting, scanning, databoxes) are not ported.
    let mut popup = Popup::new(assets.popup_config(&layouts, &tree, &mut scene)?);
    info!(
        "game UI pop-up: {}, defaultDuration {} s, skins {:?}",
        tree.nodes[popup.config.root].path,
        popup.config.default_duration,
        popup
            .config
            .skins
            .iter()
            .map(|s| &tree.nodes[s.root].name)
            .collect::<Vec<_>>()
    );
    // `OnAnalyze(analysis, verbose: true)`'s entries for `--analyze`, by
    // tech name, shown once the HUD is active.
    let mut analyze_entries: Vec<(String, Entry)> = Vec::new();
    if !analyze.is_empty() {
        let player = sn_assets::player_data(&assets)?;
        let analysis = assets.analysis_popups(&layouts, &player)?;
        let text = sn_assets::item_text(&game, &code, "English")?;
        for name in &analyze {
            let tech = text
                .tech_names
                .iter()
                .find(|(_, n)| *n == name)
                .map(|(t, _)| *t);
            match tech.and_then(|t| analysis.get(&t)) {
                Some(a) => {
                    let e = sn_assets::analysis_entry(
                        &popup.config,
                        a,
                        name,
                        &text.strings,
                        &mut scene,
                    );
                    info!(
                        "game UI pop-up: --analyze {name}: title {:?}, text {:?}, picture {}, sound {:?}",
                        e.title,
                        e.text,
                        a.unlock_popup
                            .as_ref()
                            .map_or("none (defaultSprite)".to_string(), |o| format!("{o:?}")),
                        e.sound
                    );
                    analyze_entries.push((name.clone(), e));
                }
                None => {
                    warn!("game UI pop-up: --analyze {name}: no tech type or no analysis entry")
                }
            }
        }
    }
    // The PDA's screen (M9u8a4a): `PDA.ui` instantiates `prefabScreen`;
    // here a copy grafted into the scene, its canvas at its stored size
    // and scale (Inversed with an anchor: `UpdateFrustum` is skipped).
    let pda_tree = assets.pda_screen_tree()?;
    let pda_config = assets.pda_screen_config(&layouts, &pda_tree)?;
    let pda_template = assets.ui_scene(&layouts, &pda_tree, &fonts, &tags.sorting_layers)?;
    let pda_rect = pda_template
        .stored_rect(pda_config.root)
        .cloned()
        .ok_or("uGUI_PDAScreen: no RectTransform")?;
    let pda_root = scene.graft(&pda_template, 0)?;
    let mut pda_screen = PdaScreen::new(pda_config.shifted(pda_root));
    let pda_placement = Placement {
        position: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        size: pda_rect.size_delta,
        scale: pda_rect.scale,
        scale_factor: 1.0,
    };
    info!(
        "game UI PDA screen: {} nodes grafted at {pda_root} ({}), canvas {}×{} at scale {} ({:.4} × {:.4} m); {} tabs {:?}",
        pda_tree.nodes.len(),
        scene.tree.nodes[pda_root].path,
        pda_rect.size_delta[0],
        pda_rect.size_delta[1],
        pda_rect.scale[0],
        pda_rect.size_delta[0] * pda_rect.scale[0],
        pda_rect.size_delta[1] * pda_rect.scale[1],
        pda_screen.config.tabs.len(),
        pda_screen
            .config
            .tabs
            .iter()
            .map(|t| (t.tab, t.show))
            .collect::<Vec<_>>()
    );
    // Its tab bar (M9u8a4b): `uGUI_Toolbar`, its icons made by code
    // (`ToolbarIcon` + background + foreground) with `iconMaterial`, the
    // tabs' sprites from the `Tabs` atlas; `SetTabs` as `Initialize` ends.
    let toolbar_data = assets.pda_toolbar(&layouts, &pda_tree)?;
    let mut toolbar_config = toolbar_data.config.clone();
    toolbar_config.node += pda_root;
    toolbar_config.marker = toolbar_config.marker.map(|m| m + pda_root);
    {
        let mut st = start_state.clone();
        st.apply(&pda_screen.state);
        let only_pda = HashMap::from([(pda_root, pda_placement)]);
        toolbar_config.lay_out(&scene.layout_for(&assets, &set, &only_pda, &st)?);
    }
    let toolbar_template = sn_assets::UiScene::from_code(
        "ToolbarIcon",
        sn_assets::toolbar_icon_nodes(scene.tree.nodes[toolbar_config.node].layer, &icon_material),
    )?;
    let mut tab_sprites = Vec::new();
    for tab in REGULAR_TABS {
        match item_sprites.get_no_default(sn_assets::GROUP_TAB, &format!("Tab{tab:?}")) {
            Some(s) => tab_sprites.push((
                tab,
                (
                    scene.sprite_id(&s.object),
                    [s.sprite.rect[2], s.sprite.rect[3]],
                ),
            )),
            // `SpriteManager.Get(Group.Tab, …)` gives null: no foreground.
            None => warn!("game UI PDA toolbar: Tab{tab:?}: not in the Tabs atlas"),
        }
    }
    let toolbar_sprites = ToolbarSprites {
        normal: (
            scene.sprite_id(&toolbar_data.normal.0),
            toolbar_data.normal.2,
        ),
        selected: (
            scene.sprite_id(&toolbar_data.selected.0),
            toolbar_data.selected.2,
        ),
    };
    let toolbar_node = toolbar_config.node;
    {
        let mut failed = None;
        let mut make = || match scene.graft(&toolbar_template, toolbar_node) {
            Ok(n) => ToolbarIcon {
                root: n,
                background: n + 1,
                foreground: n + 2,
            },
            Err(e) => {
                failed = Some(e);
                ToolbarIcon {
                    root: usize::MAX,
                    background: usize::MAX,
                    foreground: usize::MAX,
                }
            }
        };
        pda_screen.attach_toolbar(
            Toolbar::new(toolbar_config.clone(), toolbar_sprites),
            tab_sprites,
            &mut make,
        );
        if let Some(e) = failed {
            return Err(format!("game UI PDA toolbar: graft: {e}"));
        }
    }
    if let Some(tb) = &pda_screen.toolbar {
        for &n in &tb.events.last_sibling {
            scene.set_as_last_sibling(n);
        }
        let c = &tb.config;
        info!(
            "game UI PDA toolbar: {} icons at {:?}, bar {} wide (rule {} × {} + {} × {}), button {:?}, backgrounds {:?} / {:?}, marker {:?} (offset {:?}); tooltips (TooltipFactory.Label) and notification counts (NotificationManager, M9u8b) not ported",
            tb.icons.len(),
            tb.icons.iter().map(|i| i.root).collect::<Vec<_>>(),
            tb.width,
            tb.count,
            c.background_size[0],
            tb.count.saturating_sub(1),
            c.button_step,
            c.button_size,
            toolbar_data.normal.1,
            toolbar_data.selected.1,
            c.marker.map(|m| &scene.tree.nodes[m].path),
            c.marker_offset
        );
    }
    // The inventory tab's containers and the doll (M9u8b1):
    // `uGUI_InventoryTab` and its `Awake`s, `Start` with the player's
    // container; the locker's size and label from the lifepod.
    let item_code = sn_assets::item_code(&code)?;
    let pod = sn_assets::player_items(&assets)?;
    let mut tab_config = assets.inventory_tab_config(
        &layouts,
        &pda_tree,
        &language,
        &[pod.pod_locker.storage_label.as_str()],
    )?;
    {
        let off = pda_root;
        let c = &mut tab_config;
        for k in std::iter::once(&mut c.inventory)
            .chain(std::iter::once(&mut c.storage))
            .chain(c.torpedo.iter_mut())
        {
            k.node += off;
            k.grid = k.grid.map(|g| g + off);
        }
        c.equipment += off;
        for sl in &mut c.slots {
            sl.node += off;
            sl.background += off;
            sl.hint = sl.hint.map(|h| h + off);
        }
        c.inventory_label += off;
        c.storage_label += off;
        let mut st = start_state.clone();
        st.apply(&pda_screen.state);
        let only_pda = HashMap::from([(pda_root, pda_placement)]);
        c.lay_out(&scene.layout_for(&assets, &set, &only_pda, &st)?);
    }
    let mut inventory_tab = InventoryTab::new(tab_config);
    inventory_tab.start((item_code.inventory_size[0], item_code.inventory_size[1]));
    let locker_storage = UsedStorage::Container {
        size: (pod.pod_locker.width, pod.pod_locker.height),
        label: pod.pod_locker.storage_label.clone(),
    };
    let player_equipment = UsedStorage::Equipment(item_code.player_slots.clone());
    info!(
        "game UI PDA inventory tab: inventory {:?} ({}), {} named slots (the player's {:?}), locker {}×{} ({:?} → {:?}); the items' icons are not drawn yet (M9u8b2)",
        item_code.inventory_size,
        scene.tree.nodes[inventory_tab.config.inventory.node].path,
        inventory_tab.config.slots.len(),
        item_code.player_slots,
        pod.pod_locker.width,
        pod.pod_locker.height,
        pod.pod_locker.storage_label,
        inventory_tab.config.text(&pod.pod_locker.storage_label)
    );
    // The tab's icons (M9u8b2a): `Item Icon` / `Equipment Slot Icon` made
    // by code, the backgrounds from Resources, each tech's size and
    // background type.
    let tech_code = sn_assets::game_code(&code)?;
    let tech_data = sn_assets::tech_data(&assets)?;
    let backgrounds = assets.background_sprites()?;
    let icon_layer = scene.tree.nodes[inventory_tab.config.inventory.node].layer;
    let item_icon_template = sn_assets::UiScene::from_code(
        "Item Icon",
        sn_assets::item_icon_nodes(icon_layer, &icon_material, false),
    )?;
    let slot_icon_template = sn_assets::UiScene::from_code(
        "Equipment Slot Icon",
        sn_assets::item_icon_nodes(icon_layer, &icon_material, true),
    )?;
    info!(
        "game UI PDA inventory tab: backgrounds {:?}; bars (UI/IconBar) and grey (GRAYSCALE) not ported",
        backgrounds
            .0
            .iter()
            .map(|b| b.as_ref().map(|b| (b.name.clone(), b.size)))
            .collect::<Vec<_>>()
    );
    // The notifications (M9u8b5a): `uGUI.prefabNotificationLabel` and the
    // toolbar's anchor and offset for the count.
    let (label_nodes, label_template) =
        assets.notification_label(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    info!(
        "game UI notifications: NotificationLabel background {}, text {} (colour {:?}); toolbar anchor {} offset {:?}",
        label_nodes.background,
        label_nodes.text,
        label_nodes.text_color,
        toolbar_data.notification_anchor,
        toolbar_data.notification_offset
    );
    let mut notices = notify::Notices::new(
        label_template,
        label_nodes,
        toolbar_data.notification_anchor,
        toolbar_data.notification_offset,
    );
    let mut tooltips = tooltip::Tooltips::new(&assets, &layouts, &tree, &game, &set, &language)?;
    let mut tooltip_pose = None;
    // Dragging (M9u8b4): the icons' items by id (from the last sync) and
    // `ItemActions/Swap` at its native size (`SetNativeSize`, the drag
    // canvas's 100 pixels per unit).
    let mut dragging = drag::Dragging::default();
    let mut drag_items: BTreeMap<u64, TabItem> = BTreeMap::new();
    let mut pending_moves = Vec::new();
    let swap_sprite = item_sprites
        .get_no_default(sn_assets::GROUP_ITEM_ACTIONS, "Swap")
        .map(|s| {
            let k = 100.0 / s.sprite.pixels_to_units.max(f32::EPSILON);
            (
                scene.sprite_id(&s.object),
                [s.sprite.rect[2] * k, s.sprite.rect[3] * k],
            )
        });
    info!(
        "game UI drag: ItemActions/Swap {:?}",
        swap_sprite.map(|(_, size)| size)
    );
    let mut tab_seen = TabContents::default();
    // The census's shader sets last logged.
    let mut census_seen: Option<(Vec<String>, Vec<String>)> = None;
    let mut pda_seen_icons = PdaFlags::default();
    // The pointer last hit-tested (M9u8b3a) and the item clicks found since
    // the last build.

    let mut pending_item_clicks: Vec<(u64, u8)> = Vec::new();
    let mut pda_seen = PdaFlags::default();
    // What `SetTransition` reads, when first needed after a resize.
    let mut popup_layout: Option<PopupLayout> = None;
    // The entry shown: (title / text, shown at).
    let mut popup_shown: Option<(String, f32)> = None;
    // The death timeline's marks (what, time), for the logs.
    let mut death_marks: Vec<(&str, f32)> = Vec::new();
    let mut black_full_logged = false;
    // The icon's last change (time, icon), until its fade ends.
    let mut fading: Option<(f32, IconType)> = None;
    let (food, water) = sn_assets::survival_start(&code)?;
    info!(
        "game UI: uGUI read in {:.2} s; {} canvas scalers; HUD: {} bars, food {food}, water {water}",
        start.elapsed().as_secs_f64(),
        scalers.len(),
        hud.config.bars.len()
    );
    let camera = UiCamera::game(fov, NEAR, FAR);
    let mut sent: std::collections::HashSet<u64> = std::collections::HashSet::new();
    // Glyphs per font atlas when it was last sent.
    let mut sent_glyphs: HashMap<usize, usize> = HashMap::new();
    let mut screen: Option<[i32; 2]> = None;
    let mut placements = HashMap::new();
    let mut rules = UiState::default();
    // The frames' draw time, logged every 600 builds; the bars, each
    // second of game time.
    let (mut draw_seconds, mut draws) = (0.0, 0u32);
    let mut logged_second = -1i64;
    while let Ok(first) = requests.recv() {
        let rules_perf = crate::perf::span("game_ui::worker_rules");
        let mut resized = false;
        let mut frames = 0;
        // Every frame's rules run, in order; only the latest state is drawn.
        for request in std::iter::once(first).chain(requests.try_iter()) {
            match request {
                Request::Size(s) => {
                    resized |= screen != Some(s);
                    screen = Some(s);
                }
                Request::Frame(frame) => {
                    let (
                        mut inputs,
                        hand,
                        death_inputs,
                        keys,
                        plays,
                        pda,
                        pda_clicks,
                        tab_contents,
                    ) = *frame;
                    inputs.food = food;
                    inputs.water = water;
                    rules = hud.update(&inputs);
                    rules.apply(depth.update(&inputs));
                    let was = (
                        death.death.active,
                        death.death.fade_in,
                        death.fader.sequence.active(),
                    );
                    rules.apply(&death.update(&death_inputs));
                    death_log(
                        &death,
                        &death_inputs,
                        was,
                        &mut death_marks,
                        &mut black_full_logged,
                    );
                    // `ErrorMessage` (M9u7b).
                    let text =
                        |k: &&str| language.get(*k).cloned().unwrap_or_else(|| k.to_string());
                    let message_inputs = MessageInputs {
                        delta_time: inputs.delta_time,
                        early: keys.early.iter().map(text).collect(),
                        late: keys.late.iter().map(text).collect(),
                    };
                    let busy = !messages.messages.is_empty()
                        || messages.offset_y != 0.0
                        || !message_inputs.early.is_empty()
                        || !message_inputs.late.is_empty();
                    if busy && message_layout.is_none() {
                        let l = sn_assets::message_layout(
                            &assets,
                            &mut scene,
                            &set,
                            &placements,
                            &start_state,
                            &messages.config,
                            &message_template,
                            &scale_modes,
                            DEFAULT_UI_SCALE,
                        )?;
                        info!(
                            "game UI messages: canvas rect {:?}, entry width {}, anchor point {:?}, inverse scale {}",
                            l.canvas_rect, l.entry_width, l.anchor_base, l.inverse_scale
                        );
                        message_layout = Some(l);
                    }
                    match (busy, message_layout) {
                        (true, Some(layout)) => {
                            let mut failed = None;
                            let measure = |t: &str| {
                                message_template
                                    .preferred_size(&set, 0, t, layout.entry_width)
                                    .unwrap_or([0.0, 0.0])
                            };
                            let canvas = messages.config.canvas;
                            let mut make = || match scene.graft(&message_template, canvas) {
                                Ok(n) => n,
                                Err(e) => {
                                    failed = Some(e);
                                    usize::MAX
                                }
                            };
                            rules.apply(messages.update(
                                &message_inputs,
                                &layout,
                                &measure,
                                &mut make,
                            ));
                            if let Some(e) = failed {
                                return Err(format!("game UI messages: graft: {e}"));
                            }
                            messages_log(&messages, &mut message_log, inputs.delta_time);
                        }
                        // Nothing shown: only `PDA.time` moves (the copies
                        // stay hidden).
                        _ => {
                            messages.time += inputs.delta_time;
                            rules.apply(&messages.state);
                        }
                    }
                    // `uGUI_IconNotifier` (M9u7c2): `Play(techType, From)`
                    // from `Inventory.Pickup`, `SpriteManager.Get(techType)`.
                    let mut icon_plays = Vec::new();
                    for name in &plays {
                        match item_sprites.item_icon(name) {
                            Some(s) => {
                                let id = scene.sprite_id(&s.object);
                                icon_plays.push((id, [s.sprite.rect[2], s.sprite.rect[3]], FROM));
                                icon_names.push_back(format!("{name} ({})", s.sprite.name));
                                info!(
                                    "game UI icons at {:.3} s: Play({name}, From): sprite {:?}, queue {}",
                                    notifier.pda_time,
                                    s.sprite.name,
                                    notifier.queue.len() + icon_plays.len()
                                );
                            }
                            None => warn!("game UI icons: {name}: no sprite and no Unknown"),
                        }
                    }
                    let icons_busy = notifier.busy() || !icon_plays.is_empty();
                    if icons_busy && notifier_rect.is_none() {
                        let l = scene.layout_for(&assets, &set, &placements, &start_state)?;
                        let r = l.rect(notifier.config.canvas);
                        info!("game UI icons: canvas rect {r:?}");
                        notifier_rect = Some(r);
                    }
                    match notifier_rect {
                        Some(rect) if icons_busy => {
                            let mut failed = None;
                            let canvas = notifier.config.canvas;
                            let mut make = || match scene.graft(&icon_template, canvas) {
                                Ok(n) => Icon {
                                    root: n,
                                    foreground: n + 1,
                                },
                                Err(e) => {
                                    failed = Some(e);
                                    Icon {
                                        root: usize::MAX,
                                        foreground: usize::MAX,
                                    }
                                }
                            };
                            notifier.update(&icon_plays, inputs.delta_time, rect, &mut make);
                            if let Some(e) = failed {
                                return Err(format!("game UI icons: graft: {e}"));
                            }
                            for &n in &notifier.events.last_sibling {
                                scene.set_as_last_sibling(n);
                            }
                            icons_log(
                                &notifier,
                                &mut icon_log,
                                &mut icon_names,
                                &mut icon_last_step,
                                inputs.delta_time,
                            );
                        }
                        // Nothing queued or shown: only `PDA.time` moves.
                        _ => notifier.pda_time += inputs.delta_time,
                    }
                    rules.apply(&notifier.state);
                    // `uGUI_PopupNotification` (M9u7d1): `--analyze`'s
                    // entries once the HUD is active, then `OnUpdate` and
                    // `SetTransition`.
                    let enqueue: Vec<Entry> = if inputs.active && !analyze_entries.is_empty() {
                        let queued = popup.queue.len();
                        analyze_entries
                            .drain(..)
                            .enumerate()
                            .map(|(k, (name, e))| {
                                info!(
                                    "game UI pop-up at {:.3} s: OnAnalyze({name}), queue {}",
                                    popup.time,
                                    queued + k + 1
                                );
                                e
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let popup_busy = !popup.idle() || !enqueue.is_empty();
                    if popup_busy && popup_layout.is_none() {
                        let l = sn_assets::popup_layout(
                            &assets,
                            &mut scene,
                            &set,
                            &placements,
                            &start_state,
                            &popup.config,
                        )?;
                        info!("game UI pop-up: rect {:?}, skins {:?}", l.parent, l.skins);
                        popup_layout = Some(l);
                    }
                    match &popup_layout {
                        Some(l) if popup_busy => {
                            popup.update(&enqueue, inputs.delta_time, l);
                            popup_log(&popup, &mut popup_shown, inputs.delta_time);
                        }
                        // Nothing queued or shown: only `PDA.time` moves.
                        _ => popup.time += inputs.delta_time,
                    }
                    rules.apply(&popup.state);
                    // `uGUI_PDA` (M9u8a4a): the PDA's calls.
                    let used = if pda.with_locker {
                        &locker_storage
                    } else {
                        &player_equipment
                    };
                    pda_screen_calls(
                        &mut pda_screen,
                        &mut inventory_tab,
                        used,
                        &mut pda_seen,
                        pda,
                        inputs.time,
                    );
                    // The icons (M9u8b2a), when the contents changed or the
                    // PDA opened or closed.
                    if tab_contents != tab_seen || pda != pda_seen_icons {
                        let mut to_items = |list: &[TabEntry]| -> Vec<TabItem> {
                            list.iter()
                                .filter_map(|e| {
                                    tab_item(
                                        &mut scene,
                                        &tech_code,
                                        &tech_data,
                                        &backgrounds,
                                        &item_sprites,
                                        e,
                                    )
                                })
                                .collect()
                        };
                        let inv = to_items(&tab_contents.inventory);
                        let locker = to_items(&tab_contents.locker);
                        let equipment: Vec<(String, TabItem)> = tab_contents
                            .equipment
                            .iter()
                            .filter_map(|(slot, e)| {
                                tab_item(
                                    &mut scene,
                                    &tech_code,
                                    &tech_data,
                                    &backgrounds,
                                    &item_sprites,
                                    e,
                                )
                                .map(|i| (slot.clone(), i))
                            })
                            .collect();
                        drag_items = inv
                            .iter()
                            .chain(&locker)
                            .chain(equipment.iter().map(|(_, i)| i))
                            .map(|i| (i.id, *i))
                            .collect();
                        let mut failed = None;
                        inventory_tab.sync_items(&inv, &locker, &equipment, &mut |m| {
                            let (t, parent) = match m {
                                MakeIcon::Item(p) => (&item_icon_template, p),
                                MakeIcon::Slot(p) => (&slot_icon_template, p),
                            };
                            match scene.graft(t, parent) {
                                Ok(n) => IconNodes {
                                    root: n,
                                    background: n + 1,
                                    foreground: n + 2,
                                },
                                Err(e) => {
                                    failed = Some(e);
                                    IconNodes {
                                        root: usize::MAX,
                                        background: usize::MAX,
                                        foreground: usize::MAX,
                                    }
                                }
                            }
                        });
                        if let Some(e) = failed {
                            return Err(format!("game UI PDA inventory tab: graft: {e}"));
                        }
                        for (what, c) in [
                            ("inventory", &inventory_tab.inventory_icons),
                            ("locker", &inventory_tab.storage_icons),
                        ] {
                            let ev = &c.events;
                            if !ev.added.is_empty()
                                || !ev.removed.is_empty()
                                || !ev.moved.is_empty()
                            {
                                info!(
                                    "game UI PDA inventory tab at {:.3} s: {what} icons: {} shown, added {:?}, removed {:?}, moved {:?}, {} made by code",
                                    inputs.time,
                                    c.icons.len(),
                                    ev.added,
                                    ev.removed,
                                    ev.moved,
                                    ev.made
                                );
                            }
                        }
                        let slots: Vec<(&String, Option<u64>)> = inventory_tab
                            .slot_icons
                            .iter()
                            .map(|(k, v)| (k, v.item))
                            .collect();
                        if !slots.is_empty() {
                            info!(
                                "game UI PDA inventory tab at {:.3} s: slot icons {slots:?}",
                                inputs.time
                            );
                        }
                        tab_seen = tab_contents.clone();
                        pda_seen_icons = pda;
                    }
                    rules.apply(&pda_screen.state);
                    rules.apply(&inventory_tab.state);
                    rules.apply(&dragging.state);
                    // The pointer on the screen (M9u8a4b's toolbar, M9u8b3a's
                    // tab): the hover when the pointer, the icons or the PDA
                    // change; the clicks.
                    let hover_on =
                        pda.focused && (pda_clicks.pointer.is_some() || pda_clicks.hover.is_some());
                    tooltip_pose = None;
                    if !pda.focused || pda_screen.tab_open != Some(sn_ui::pda::Tab::Inventory) {
                        dragging.stop(&mut inventory_tab, inputs.time);
                    }
                    if !pda_clicks.clicks.is_empty() || !pda_clicks.drag.is_empty() || hover_on {
                        if let (Some((position, rotation)), Some(size)) = (pda_clicks.pose, screen)
                        {
                            let mut st = start_state.clone();
                            st.apply(&rules);
                            let mut ps = placements.clone();
                            let placed = Placement {
                                position,
                                rotation,
                                ..pda_placement
                            };
                            ps.insert(pda_root, placed);
                            let l = scene.layout_for(&assets, &set, &ps, &st)?;
                            let canvas = tooltip::pointer(
                                &inventory_tab,
                                &l,
                                pda_root,
                                &placed,
                                &camera,
                                size,
                                &pda_clicks,
                            );
                            let over = (hover_on
                                && pda_screen.tab_open == Some(sn_ui::pda::Tab::Inventory))
                            .then(|| canvas.and_then(|c| inventory_tab.hit(&l, pda_root, c)))
                            .flatten()
                            .and_then(|h| h.item());
                            // The left button's press, drag and release
                            // (M9u8b4).
                            let dragged = dragging.update(
                                &drag::DragFrame {
                                    l: &l,
                                    root: pda_root,
                                    placed: &placed,
                                    camera: &camera,
                                    screen: size,
                                    canvas,
                                    inputs: &pda_clicks.drag,
                                    dt: inputs.delta_time,
                                    time: inputs.time,
                                },
                                &mut inventory_tab,
                                &tab_contents,
                                &drag_items,
                                swap_sprite,
                                &mut scene,
                                &slot_icon_template,
                            )?;
                            pending_moves.extend(dragged.moves);
                            pending_item_clicks.extend(dragged.item_clicks);
                            // No hover and no tooltip while dragging.
                            let over = over.filter(|_| !dragging.active());
                            let entry = over.and_then(|id| {
                                tab_contents
                                    .inventory
                                    .iter()
                                    .chain(&tab_contents.locker)
                                    .chain(tab_contents.equipment.iter().map(|(_, e)| e))
                                    .find(|e| e.id == id)
                            });
                            let slots = entry.map(|e| e.compatible.as_slice()).unwrap_or_default();
                            if !dragging.active()
                                && !dragging.hover_held()
                                && inventory_tab.set_hovered(over.map(|i| (i, slots)))
                            {
                                info!(
                                    "game UI PDA inventory tab at {:.3} s: hovered {over:?}, slots lit {:?}",
                                    inputs.time, inventory_tab.highlighted
                                );
                            }
                            tooltip_pose = tooltips.update(
                                &assets,
                                &layouts,
                                &catalog,
                                &mut scene,
                                &set,
                                &language,
                                entry,
                                canvas,
                                &l,
                                pda_root,
                                over.and_then(|id| tooltip::item_node(&inventory_tab, id))
                                    .unwrap_or(pda_root),
                                &placed,
                                &camera,
                                &st,
                                &mut placements,
                                &mut rules,
                                inputs.time,
                            )?;
                            for click in pda_clicks.clicks.iter().chain(&dragged.screen_clicks) {
                                if let Some(c) = screen_click(
                                    &mut pda_screen,
                                    &mut inventory_tab,
                                    &l,
                                    pda_root,
                                    &placed,
                                    &camera,
                                    size,
                                    *click,
                                    inputs.time,
                                ) {
                                    pending_item_clicks.push(c);
                                }
                            }
                        }
                    }
                    if !hover_on {
                        inventory_tab.set_hovered(None);
                    }
                    if tooltip_pose.is_none() {
                        tooltips.clear(&mut rules, inputs.time);
                    }
                    rules.apply(&pda_screen.state);
                    rules.apply(&inventory_tab.state);
                    rules.apply(&dragging.state);
                    // The notifications (M9u8b5a), after the tab's state.
                    notices.update(
                        &mut scene,
                        &inventory_tab,
                        pda_screen.toolbar.as_ref(),
                        &tab_contents,
                        inputs.delta_time,
                        inputs.time,
                        &|labels| {
                            let mut st = start_state.clone();
                            st.apply(&rules);
                            st.apply(labels);
                            st
                        },
                    )?;
                    rules.apply(&notices.state);
                    let before = reticle.icon_type;
                    rules.apply(reticle.update(&reticle_inputs(
                        &reticle,
                        &left_hand.text,
                        &inputs,
                        &hand,
                    )));
                    if reticle.icon_type != before {
                        info!(
                            "game UI reticle at {:.3} s: icon {:?} → {:?}; hand {:?}, subscript {:?}",
                            inputs.time,
                            before,
                            reticle.icon_type,
                            reticle.state.text.get(&reticle.config.texts[HAND]),
                            reticle
                                .state
                                .text
                                .get(&reticle.config.texts[HAND_SUBSCRIPT])
                        );
                        // From the start of the change's frame (its step
                        // already moved the fade).
                        fading = Some((inputs.time - inputs.delta_time, reticle.icon_type));
                    }
                    if let Some((at, icon)) = fading {
                        let t = icon_alpha(&reticle, icon);
                        if icon == IconType::None || t >= 1.0 {
                            // The rule: done on the first frame whose steps
                            // add up to 0.1 s (at once to `None`).
                            let took = inputs.time - at;
                            let rule = if icon == IconType::None { 0.0 } else { 0.1 };
                            let ok = took + 1e-4 >= rule && took < rule + inputs.delta_time + 1e-4;
                            info!(
                                "game UI reticle: {icon:?} fully in after {took:.4} s ({}: rule {rule} s, last step {:.4} s)",
                                if ok { "ok" } else { "DIFFERS" },
                                inputs.delta_time
                            );
                            fading = None;
                        }
                    }
                    frames += 1;
                    let second = inputs.time.floor() as i64;
                    if second != logged_second {
                        logged_second = second;
                        info!("game UI bars at {second} s: {}", bars_line(&hud, &inputs));
                        info!(
                            "game UI death at {second} s: suffocation {:?} → overlay {:.4}; black {:.4} (enabled {:?}, active {}, fading in {}); respawn fader {:.4}",
                            death.overlay_values.first().copied().flatten(),
                            death.overlay_alpha(0),
                            death.death.alpha,
                            death.death.enabled,
                            death.death.active,
                            death.death.fade_in,
                            death.fader.alpha.unwrap_or(0.0)
                        );
                        info!(
                            "game UI depth at {second} s: {}",
                            depth_line(&depth, &inputs)
                        );
                        info!(
                            "game UI reticle at {second} s: icon {:?}, hand {:?}, subscript {:?}",
                            reticle.icon_type,
                            reticle.state.text.get(&reticle.config.texts[HAND]),
                            reticle
                                .state
                                .text
                                .get(&reticle.config.texts[HAND_SUBSCRIPT])
                        );
                    }
                }
            }
        }
        drop(rules_perf);
        let Some(size) = screen else {
            continue;
        };
        if resized {
            message_layout = None;
            notifier_rect = None;
            popup_layout = None;
            placements.clear();
            for (node, s) in &scalers {
                match place(s, &camera, fov, size, DEFAULT_UI_SCALE) {
                    Ok(p) => {
                        placements.insert(*node, p);
                    }
                    Err(e) => warn!("game UI: {}: {e}", tree.nodes[*node].path),
                }
            }
            placements.insert(pda_root, pda_placement);
        } else if frames == 0 {
            continue;
        }
        let _perf = crate::perf::span("game_ui::worker_build");
        let begin = Instant::now();
        let mut state = start_state.clone();
        state.apply(&rules);
        let mut d = {
            let _perf = crate::perf::span("game_ui::scene_draw");
            scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?
        };
        // The dragged icon over the rest (M9u8b4).
        dragging.draw_last(&mut d);
        if let Some(o) = &overlay {
            d.textures.insert(sn_assets::texture_key(o), o.clone());
        }
        // The scene's tree: the stored nodes and the copies grafted since.
        let mut build = convert(
            &scene.tree,
            &d,
            size,
            fov,
            placements.clone(),
            resized,
            &mut census_seen,
        );
        build.pda_root = Some(pda_root);
        build.tooltip = tooltip_pose;
        build.item_moves = std::mem::take(&mut pending_moves);
        build.item_clicks = std::mem::take(&mut pending_item_clicks);
        for (key, object) in &d.textures {
            if sent.contains(key) {
                continue;
            }
            match load_texture(&assets, *key, object) {
                Ok(t) => {
                    sent.insert(*key);
                    build.textures.push(t);
                }
                Err(e) => warn!("game UI: texture {key:x}: {e}"),
            }
        }
        // Font atlases with new glyphs (sent whole; their texels are
        // already bottom row first).
        for (font, a) in atlases.atlases.iter().enumerate() {
            if a.glyphs.is_empty() || sent_glyphs.get(&font) == Some(&a.glyphs.len()) {
                continue;
            }
            sent_glyphs.insert(font, a.glyphs.len());
            info!(
                "game UI: font atlas {} sent: {} glyphs, {}×{} ({} made so far in {:.3} s; atlas full {})",
                fonts.assets[font].name,
                a.glyphs.len(),
                a.width,
                a.height,
                atlases.made,
                atlases.seconds,
                atlases.full
            );
            build.textures.push(UiTexture {
                key: sn_assets::font_texture_key(font),
                width: a.width.max(0) as u32,
                height: a.height.max(0) as u32,
                data: a.texels.clone(),
                format: TexelFormat::R8,
                repeat: false,
                point: false,
                alpha_only: true,
            });
        }
        build.seconds = begin.elapsed().as_secs_f64();
        if resized {
            info!(
                "game UI: drawn for {}×{} in {:.3} ms",
                size[0],
                size[1],
                build.seconds * 1000.0
            );
        } else {
            draw_seconds += build.seconds;
            draws += 1;
            if draws == 600 {
                info!(
                    "game UI: frames drawn on the worker in {:.3} ms each (mean of 600)",
                    draw_seconds * 1000.0 / 600.0
                );
                (draw_seconds, draws) = (0.0, 0);
            }
        }
        build.resized = resized;
        if builds.send(Ok(build)).is_err() {
            break;
        }
    }
    Ok(())
}

/// The messages' changes as they happen (M9u7b): each added, renewed and
/// gone, with how long it was shown against `ErrorMessage`'s times (gone
/// on the first frame after its end, or the next one when the message
/// before it went in the same frame).
fn messages_log(m: &Messages, log: &mut HashMap<usize, (String, f32, f32)>, dt: f32) {
    let t = m.config.times;
    for msg in &m.messages {
        let shown = m.state.text.get(&msg.entry).cloned().unwrap_or_default();
        match log.get_mut(&msg.entry) {
            None => {
                info!(
                    "game UI message at {:.3} s: {shown:?} (entry {}, {} shown, pool {} of {})",
                    m.time,
                    msg.entry,
                    m.messages.len(),
                    m.pool.len(),
                    m.entries.len()
                );
                log.insert(msg.entry, (shown, m.time, m.time));
            }
            Some(e) if e.0 != shown => {
                info!("game UI message at {:.3} s: renewed as {shown:?}", m.time);
                (e.0, e.2) = (shown, m.time);
            }
            _ => {}
        }
    }
    let gone: Vec<usize> = log
        .keys()
        .copied()
        .filter(|e| !m.messages.iter().any(|msg| msg.entry == *e))
        .collect();
    for e in gone {
        let Some((text, first, last)) = log.remove(&e) else {
            continue;
        };
        // First shown: fly-in + delay + fade-out + invisible; renewed:
        // without the fly-in.
        let rule = if last > first {
            t.delay + t.fade_out + t.invisible
        } else {
            t.fly_in + t.delay + t.fade_out + t.invisible
        };
        let took = m.time - last;
        let ok = took > rule - 1e-4 && took <= rule + 2.0 * dt + 1e-4;
        info!(
            "game UI message gone at {:.3} s: {text:?} after {took:.4} s since shown or renewed ({}: rule {rule} s + under two frames, last step {dt:.4} s)",
            m.time,
            if ok { "ok" } else { "DIFFERS" }
        );
    }
}

/// The icons' lives (M9u7c2): each shown (its sprite, `PDA.time`) and
/// released, against its animator's duration: t steps by each frame's
/// `deltaTime` ÷ duration from the shown frame on, and the icon goes on
/// the first frame that finds t = 1, so the steps up to the frame before
/// the release add up to at least the duration and less than it + the
/// last of them. `names`: the queued requests' sprites.
fn icons_log(
    n: &Notifier,
    log: &mut HashMap<usize, (String, f32, f32)>,
    names: &mut std::collections::VecDeque<String>,
    last: &mut f32,
    dt: f32,
) {
    for icon in &n.events.released {
        let Some((name, at, first)) = log.remove(&icon.root) else {
            continue;
        };
        let rule = n.config.animators[FROM].duration;
        // From the shown frame's step to the previous frame's.
        let steps = (n.pda_time - dt) - (at - first);
        let ok = steps + 1e-4 >= rule && steps < rule + *last + 1e-4;
        info!(
            "game UI icon at {:.3} s: released {name} (icon {}) {:.4} s after shown; steps until the frame before {steps:.4} s ({}: duration {rule} s, under one more step of {:.4} s)",
            n.pda_time,
            icon.root,
            n.pda_time - at,
            if ok { "ok" } else { "DIFFERS" },
            *last
        );
    }
    for &(icon, at) in &n.events.shown {
        let name = names.pop_front().unwrap_or_default();
        info!(
            "game UI icon at {at:.3} s: shown {name} (icon {}, {} active, queue {}, pool {} of {})",
            icon.root,
            n.active.len(),
            n.queue.len(),
            n.pool.len(),
            n.icons.len()
        );
        log.insert(icon.root, (name, at, dt));
    }
    *last = dt;
}

/// The pop-up's life (M9u7d2): each entry shown, each phase, the sounds
/// not ported, and when one is done, how long it was up against the sum
/// of `timeIn`, its duration and `timeOut` (done on the first `OnUpdate`
/// past it, so under one step more).
fn popup_log(p: &Popup, shown: &mut Option<(String, f32)>, dt: f32) {
    // In `OnUpdate`'s order: the phase (the last one done), then the next
    // one shown.
    for (phase, at) in &p.events.phases {
        info!(
            "game UI pop-up at {at:.3} s: phase {phase:?}, group alpha {}",
            if (Phase::In..=Phase::Out).contains(phase) {
                1
            } else {
                0
            }
        );
        if *phase != Phase::Done {
            continue;
        }
        let Some((what, from)) = shown.take() else {
            continue;
        };
        let took = at - from;
        let rule = TIME_IN + p.config.default_duration + TIME_OUT;
        let ok = took + 1e-4 >= rule && took <= rule + dt + 1e-4;
        info!(
            "game UI pop-up at {at:.3} s: {what} done after {took:.4} s ({}: {TIME_IN} + {} + {TIME_OUT} s + under a step, step {dt:.4} s)",
            if ok { "ok" } else { "DIFFERS" },
            p.config.default_duration
        );
    }
    for (e, at) in &p.events.shown {
        info!(
            "game UI pop-up at {at:.3} s: shown {:?} / {:?} ({}), {} for {} s, queue {}",
            e.title,
            e.text,
            if e.sprite.is_some() {
                "its picture"
            } else {
                "defaultSprite"
            },
            if e.skin == sn_ui::popup::UNLOCK {
                "Unlock"
            } else {
                "Journal"
            },
            e.duration,
            p.queue.len()
        );
        *shown = Some((format!("{:?} / {:?}", e.title, e.text), *at));
    }
    for s in &p.events.sounds {
        info!("game UI pop-up: sound not ported: {s}");
    }
}

/// `uGUI_PDA`'s calls from the PDA's flags' changes (M9u8a4a), logged:
/// `OnOpenPDA` (`using_pda` up), `OnPDAOpened` (focused), `OnClosePDA`
/// (`using_pda` down), `OnPDAClosed` (the model off).
fn pda_screen_calls(
    s: &mut PdaScreen,
    tab: &mut InventoryTab,
    used: &UsedStorage,
    seen: &mut PdaFlags,
    now: PdaFlags,
    time: f32,
) {
    let shown = |s: &PdaScreen| {
        let c = &s.config;
        let alpha = |n: usize| s.state.group_alpha.get(&n).copied().unwrap_or(1.0);
        let tabs: Vec<String> = c
            .tabs
            .iter()
            .filter(|t| match t.show {
                sn_ui::pda::TabShow::Content(n) => alpha(n) > 0.0,
                sn_ui::pda::TabShow::Object(n) => s.state.active.get(&n) == Some(&true),
            })
            .map(|t| format!("{:?}", t.tab))
            .collect();
        format!(
            "canvas alpha {}, content {}, toolbar {}; tabs shown {tabs:?} of {}",
            alpha(c.canvas_group),
            alpha(c.content),
            alpha(c.toolbar_group),
            c.tabs.len()
        )
    };
    if now.using && !seen.using {
        // `uGUI_PDA.OnOpenPDA`: each tab's `OnOpenPDA`, then `OpenTab`.
        tab.on_open_pda(used.clone());
        info!(
            "game UI PDA inventory tab at {time:.3} s: OnOpenPDA with {}: storage label {:?}",
            match used {
                UsedStorage::Equipment(slots) =>
                    format!("the equipment ({} slots shown)", slots.len()),
                UsedStorage::Container { size, .. } =>
                    format!("the locker ({}×{})", size.0, size.1),
            },
            tab.state.text.get(&tab.config.storage_label)
        );
        s.on_open_pda(None);
        info!(
            "game UI PDA screen at {time:.3} s: OnOpenPDA, tab {:?}: {}",
            s.tab_open,
            shown(s)
        );
    }
    if now.focused && !seen.focused {
        s.on_pda_opened();
        info!("game UI PDA screen at {time:.3} s: OnPDAOpened");
    }
    if !now.using && seen.using {
        s.on_close_pda();
        tab.on_close_pda();
        info!(
            "game UI PDA screen at {time:.3} s: OnClosePDA: {}",
            shown(s)
        );
    }
    if !now.active && seen.active {
        s.on_pda_closed();
        info!(
            "game UI PDA screen at {time:.3} s: OnPDAClosed: {}",
            shown(s)
        );
    }
    *seen = now;
}

/// An item as the tab's icon shows it (M9u8b2a): its sprite
/// (`SpriteManager.Get(techType)`, `Unknown` if none) and background
/// (`GetBackground`, by `TechData`'s type). `None` (logged) without either.
fn tab_item(
    scene: &mut sn_assets::UiScene,
    code: &sn_assets::GameCode,
    data: &sn_assets::TechData,
    backgrounds: &sn_assets::BackgroundSprites,
    sprites: &sn_assets::Sprites,
    e: &TabEntry,
) -> Option<TabItem> {
    let name = code.tech_name(e.tech).unwrap_or("");
    let (_, bt) = sn_assets::tech_icon(data, &code.tech_defaults, e.tech);
    let (Some(fg), Some(bg)) = (sprites.item_icon(name), backgrounds.get(bt)) else {
        warn!(
            "game UI PDA inventory tab: {name} ({}): no sprite or background",
            e.tech
        );
        return None;
    };
    Some(TabItem {
        id: e.id,
        size: e.size,
        at: e.at,
        background: (scene.sprite_id(&bg.object), bg.size),
        foreground: (
            scene.sprite_id(&fg.object),
            [fg.sprite.rect[2], fg.sprite.rect[3]],
        ),
    })
}

/// A click on the PDA screen (M9u8a4b, M9u8b3a): `GraphicRaycaster`'s ray
/// from the UI camera through the pixel onto the canvas (placed at
/// `placed`). The toolbar's icons first (`uGUI_Toolbar` →
/// `uGUI_PDA.OnToolbarClick`), then, on the inventory tab, its item icons
/// and slots (`OnPointerClick` → `uGUI_InventoryTab.OnPointerClick`: the
/// item and button returned for the main thread's action;
/// `ExtinguishSlots`). `ui --pda` and `ui --inventory` found nothing drawn
/// over these. A scripted click starts from its target's centre projected
/// to the screen.
#[allow(clippy::too_many_arguments)]
fn screen_click(
    s: &mut PdaScreen,
    tab: &mut InventoryTab,
    l: &sn_ui::layout::Tree,
    root: usize,
    placed: &Placement,
    camera: &UiCamera,
    screen: [i32; 2],
    click: ScreenClick,
    time: f32,
) -> Option<(u64, u8)> {
    use sn_ui::draw::{apply, invert, to_ancestor};
    let tb = s.toolbar.as_ref()?;
    let bar = tb.config.node;
    // A node's centre on the screen.
    let project = |node: usize| -> Option<[f32; 2]> {
        let r = l.rect(node);
        let c = apply(
            &to_ancestor(l, node, root),
            [r.x + 0.5 * r.width, r.y + 0.5 * r.height, 0.0],
        );
        camera.project(apply(&placed.world_from_canvas(), c), screen)
    };
    let target = match click {
        ScreenClick::At { pixel, button } => Some((pixel, button)),
        ScreenClick::Icon(i) => tb
            .icons
            .get(i)
            .filter(|_| i < tb.count)
            .and_then(|icon| project(icon.root))
            .map(|p| (p, 0)),
        ScreenClick::Item { id, button } => {
            let node = tab
                .storage_icons
                .icons
                .get(&id)
                .or_else(|| tab.inventory_icons.icons.get(&id))
                .map(|(i, _)| i.root)
                .or_else(|| {
                    let slot = tab
                        .slot_icons
                        .iter()
                        .find(|(_, v)| v.item == Some(id))
                        .map(|(k, _)| k.clone())?;
                    tab.config
                        .slots
                        .iter()
                        .find(|x| x.slot == slot)
                        .map(|x| x.node)
                });
            node.and_then(project).map(|p| (p, button))
        }
    };
    let Some((pixel, button)) = target else {
        info!("game UI PDA screen at {time:.3} s: {click:?}: no such target on screen");
        return None;
    };
    let Some(canvas) = placed.ray_point(camera.position, camera.screen_ray(pixel, screen)) else {
        info!("game UI PDA screen at {time:.3} s: click at {pixel:?} misses the screen");
        return None;
    };
    let in_bar = invert(&to_ancestor(l, bar, root))
        .map(|m| apply(&m, [canvas[0], canvas[1], 0.0]))
        .and_then(|p| tb.hit([p[0], p[1]], l.rect(bar)));
    let at = format!(
        "{click:?}: pixel ({:.1}, {:.1}), canvas ({:.1}, {:.1})",
        pixel[0], pixel[1], canvas[0], canvas[1]
    );
    if let Some(index) = in_bar {
        let before = s.tab_open;
        let opened = s.on_toolbar_click(index, button);
        info!(
            "game UI PDA toolbar at {time:.3} s: {at}: icon {index}, button {button}; {}",
            if opened {
                format!(
                    "OpenTab {:?} → {:?}{}",
                    before,
                    s.tab_open,
                    match s.tab_open {
                        Some(sn_ui::pda::Tab::Inventory) | None => "",
                        // Their stored content only.
                        Some(_) => " (its content is not ported: drawn as stored)",
                    }
                )
            } else {
                "nothing opened".into()
            }
        );
        return None;
    }
    let hit = (s.tab_open == Some(sn_ui::pda::Tab::Inventory))
        .then(|| tab.hit(l, root, canvas))
        .flatten();
    info!("game UI PDA inventory tab at {time:.3} s: {at}: {hit:?}, button {button}");
    let id = hit?.item()?;
    tab.on_pointer_click();
    Some((id, button))
}

/// The death timeline's changes as they happen (M9u6e1), each against
/// the rules' times: the vignette's fade out 6 s after the event, the
/// respawn fader clear `fadeInTime` after `Hide`.
fn death_log(
    d: &DeathScreens,
    i: &DeathInputs,
    (was_active, was_in, was_fading): (bool, bool, bool),
    marks: &mut Vec<(&'static str, f32)>,
    black_full_logged: &mut bool,
) {
    let mark = |marks: &mut Vec<(&'static str, f32)>, what: &'static str| {
        info!(
            "game UI death at {:.3} s: {what}; overlay {:.4}, black {:.4}, fader {:.4}",
            i.time,
            d.overlay_alpha(0),
            d.death.alpha,
            d.fader.alpha.unwrap_or(0.0)
        );
        marks.push((what, i.time));
    };
    let since = |marks: &[(&'static str, f32)], what: &str| {
        marks
            .iter()
            .rev()
            .find(|(w, _)| *w == what)
            .map(|(_, t)| i.time - t)
    };
    if i.killed {
        mark(marks, "OnKill");
    }
    if let Some(kind) = i.vignette {
        mark(
            marks,
            match kind {
                DeathType::FadeToBlack => "TriggerDeathVignette(FadeToBlack)",
                DeathType::CutToBlack => "TriggerDeathVignette(CutToBlack)",
            },
        );
        *black_full_logged = false;
    }
    if i.respawn_show {
        mark(marks, "respawning.Show");
    }
    if i.respawn_hide {
        mark(marks, "respawning.Hide");
    }
    if !was_active && d.death.active {
        mark(marks, "BeginDeathFade");
    }
    if d.death.alpha >= 1.0 && !*black_full_logged {
        *black_full_logged = true;
        mark(marks, "black");
    }
    if was_in && !d.death.fade_in {
        let after = since(marks, "TriggerDeathVignette(FadeToBlack)")
            .or_else(|| since(marks, "TriggerDeathVignette(CutToBlack)"));
        let rule = d.config.vignette.fade_out_delay;
        let ok = after.is_some_and(|a| a + 1e-4 >= rule && a < rule + i.delta_time + 1e-3);
        info!(
            "game UI death: BeginFadeOut {after:?} s after the event ({}: rule {rule} s + under a frame, last step {:.4} s)",
            if ok { "ok" } else { "DIFFERS" },
            i.delta_time
        );
        mark(marks, "BeginFadeOut");
    }
    if was_active && !d.death.active {
        mark(marks, "ResetOverlay");
    }
    if was_fading && !d.fader.sequence.active() && d.fader.alpha == Some(0.0) {
        if let Some(after) = since(marks, "respawning.Hide") {
            let rule = d.config.loading_background.fade_in_time;
            // The sequence ends on the update after `t` reaches 0.
            let ok = after + 1e-4 >= rule && after < rule + 2.0 * i.unscaled_delta_time + 1e-3;
            info!(
                "game UI death: respawn fader clear {after:.4} s after Hide ({}: rule {rule} s + under two frames, last step {:.4} s)",
                if ok { "ok" } else { "DIFFERS" },
                i.unscaled_delta_time
            );
            mark(marks, "fader clear");
        }
    }
}

/// Each bar's input, shown value (`_Value`) and text, for the log.
fn bars_line(hud: &Hud, i: &HudInputs) -> String {
    let mut out = format!(
        "active {}; oxygen {:.2}/{}, health {:.1}/{}, food {}, water {}",
        hud.active, i.oxygen, i.oxygen_capacity, i.health, i.max_health, i.food, i.water
    );
    for (b, s) in hud.config.bars.iter().zip(&hud.bars) {
        out += &format!(
            "; {:?} {} \"{}\" pulse {:.3} s at {:.3}",
            b.kind,
            s.value
                .map_or("-".to_string(), |v| format!("{:.4}", b.circular.value(v))),
            s.text.as_deref().unwrap_or("-"),
            s.pulse_tween.duration(),
            s.normalized
        );
    }
    out
}

/// Unity's `eulerAngles.y` (degrees, 0…360) of a Bevy rotation: the
/// heading of the Unity forward (+z, Bevy −z), `atan2(R[0][2], R[2][2])`
/// in Unity's decomposition, whatever the roll.
pub(crate) fn euler_y(rotation: Quat) -> f32 {
    let f = rotation * Vec3::NEG_Z;
    // Unity's z is Bevy's −z.
    let y = f.x.atan2(-f.z).to_degrees();
    if y < 0.0 { y + 360.0 } else { y }
}

/// The depth input, its floor, the mode and the shown text (M9u6c1), for
/// the log.
/// The reticle's inputs from the frame: the hover's texts (the hand text
/// through `GetText` with the left hand's button) and icon. The game's
/// `AvatarInputHandler.IsEnabled()` is its cursor lock, on in play and off
/// while the PDA (or a menu, not ported) is open: here, the PDA closed.
/// Our window's own mouse grab is not that lock (a scripted run never
/// grabs it). `GUIHand` hovers only while it is on.
fn reticle_inputs(r: &HandReticle, button: &str, i: &HudInputs, hand: &HandFrame) -> ReticleInputs {
    let mut out = ReticleInputs {
        delta_time: i.delta_time,
        hidden: i.pda_open,
        ..ReticleInputs::default()
    };
    if let (Some(h), false) = (&hand.hover, i.pda_open) {
        out.texts[HAND] = get_text(&r.config, &h.text, h.button.then_some(button));
        out.texts[HAND_SUBSCRIPT] = h.subscript.clone();
        if let Some(icon) = h.icon {
            out.icon = icon;
        }
    }
    out
}

/// The alpha the icon's graphics were last given (1: never faded).
fn icon_alpha(r: &HandReticle, icon: IconType) -> f32 {
    r.config
        .icons
        .iter()
        .find(|i| i.kind == icon)
        .and_then(|i| i.graphics.first())
        .and_then(|g| r.state.renderer_alpha.get(g).copied())
        .unwrap_or(1.0)
}

fn depth_line(d: &DepthCompass, i: &HudInputs) -> String {
    let c = &d.config;
    let text = |n: usize| d.state.text.get(&n).map_or("-", String::as_str);
    let shown = d.state.enabled.get(&c.depth_text).copied().unwrap_or(false);
    format!(
        "input {:.3} m, floor {}, mode {:?}, shown {shown}, text \"{}\" \"{}\", compass equipped {} shown {}, yaw {:.3}° / 360 = {:.5}, direction {:.5}",
        i.depth,
        sn_ui::depth::floor_depth(i.depth),
        d.mode,
        text(c.depth_text),
        text(c.suffix_text),
        i.compass,
        d.compass_visible,
        i.yaw,
        sn_ui::compass::wrap(i.yaw / 360.0),
        d.direction
    )
}

/// A texture decoded to RGBA8, bottom row first.
fn load_texture(
    assets: &sn_assets::Assets,
    key: u64,
    object: &sn_assets::ObjectRef,
) -> Result<UiTexture, String> {
    let t = assets.texture(object)?;
    let tex = &t.texture;
    let rgba = tex.decode_rgba(&t.data).map_err(|e| e.to_string())?;
    let (w, h) = (tex.width.max(0) as usize, tex.height.max(0) as usize);
    if rgba.len() < w * h * 4 {
        return Err(format!("{} bytes for {w}×{h}", rgba.len()));
    }
    // `decode_rgba` gives the top row first; Unity's UVs count from the
    // bottom.
    let mut flipped = Vec::with_capacity(w * h * 4);
    for row in (0..h).rev() {
        flipped.extend_from_slice(&rgba[row * w * 4..(row + 1) * w * 4]);
    }
    Ok(UiTexture {
        key,
        width: w as u32,
        height: h as u32,
        data: flipped,
        format: if tex.color_space == 1 {
            TexelFormat::RgbaSrgb
        } else {
            TexelFormat::Rgba
        },
        repeat: tex.wrap[0] == 0,
        point: tex.filter_mode == 0,
        alpha_only: tex.format == 1,
    })
}

/// What each shader's draws became, for the log.
#[derive(Default)]
struct Census {
    drawn: BTreeMap<String, usize>,
    as_default: BTreeMap<String, usize>,
    texts: usize,
    depth_clear: usize,
    without_placement: usize,
}

/// The draw list as vertex bytes, indices and draws.
fn convert(
    tree: &sn_assets::UiTree,
    d: &sn_assets::UiDraw,
    screen: [i32; 2],
    field_of_view: f32,
    placements: HashMap<usize, Placement>,
    log: bool,
    seen: &mut Option<(Vec<String>, Vec<String>)>,
) -> UiBuild {
    let _perf = crate::perf::span("game_ui::convert");
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut draws = Vec::new();
    let mut census = Census::default();
    // Per draw group: draws, vertices, textures.
    let mut groups: BTreeMap<String, (usize, usize, std::collections::BTreeSet<u64>)> =
        BTreeMap::new();
    for it in &d.list.items {
        let Some(m) = d.materials.get(it.material) else {
            continue;
        };
        if m.shader == "uGUI/DepthClear" {
            census.depth_clear += 1;
            continue;
        }
        if !placements.contains_key(&it.root) {
            census.without_placement += 1;
            continue;
        }
        let mut flags = 0;
        let mut blend = ALPHA_BLEND;
        let clip = it.clip.map(|r| [r.x, r.y, r.x + r.width, r.y + r.height]);
        let mut text = None;
        let mut bar = None;
        let mut notification = [0.0; 4];
        let mut overlay = 0;
        match m.shader.as_str() {
            "TextMeshPro/Distance Field" => {
                census.texts += 1;
                blend = PREMULTIPLIED_BLEND;
                if clip.is_some() {
                    flags |= TEXT_CLIP;
                }
                if m.has_keyword("UNDERLAY_ON") {
                    flags |= TEXT_UNDERLAY;
                }
                if it.texture & (1 << 40) == 0 {
                    // Not a `font_texture_key`.
                    flags |= TEXT_ALPHA_ATLAS;
                }
                for t in ["_FaceTex", "_OutlineTex"] {
                    if m.textures.get(t).is_some_and(|k| *k != 0) {
                        *census
                            .as_default
                            .entry(format!("{} {t} (drawn white)", m.name))
                            .or_default() += 1;
                    }
                }
                let f = |k: &str| m.float(k);
                let c =
                    |k: &str, d: [f32; 4]| linear(m.colors.get(k).copied().unwrap_or(d)).to_array();
                text = Some(TextValues {
                    face_color: c("_FaceColor", [1.0; 4]),
                    outline_color: c("_OutlineColor", [0.0, 0.0, 0.0, 1.0]),
                    underlay_color: c("_UnderlayColor", [0.0, 0.0, 0.0, 0.5]),
                    face: [
                        f("_FaceDilate"),
                        f("_OutlineWidth"),
                        f("_OutlineSoftness"),
                        f("_ScaleRatioA"),
                    ],
                    weight: [
                        f("_WeightNormal"),
                        f("_WeightBold"),
                        f("_GradientScale"),
                        f("_Sharpness"),
                    ],
                    scale: [f("_ScaleX"), f("_ScaleY"), f("_PerspectiveFilter")],
                    underlay: [
                        f("_UnderlayOffsetX"),
                        f("_UnderlayOffsetY"),
                        f("_UnderlayDilate"),
                        f("_UnderlaySoftness"),
                    ],
                    atlas: [f("_ScaleRatioC"), f("_TextureWidth"), f("_TextureHeight")],
                    offset: [
                        f("_VertexOffsetX"),
                        f("_VertexOffsetY"),
                        f("_MaskSoftnessX"),
                        f("_MaskSoftnessY"),
                    ],
                });
                *census.drawn.entry(m.shader.clone()).or_default() += 1;
            }
            sn_ui::bar::SHADER => {
                let f = |k: &str| m.float(k);
                let v = |k: &str| m.colors.get(k).copied().unwrap_or([0.0; 4]);
                let mut bar_flags = 0;
                for (k, bit) in [("BORDER", 1), ("OVERLAY", 2), ("ROUND_CORNERS", 4)] {
                    if m.has_keyword(k) {
                        bar_flags |= bit;
                    }
                }
                let shift = v("_OverlayShift");
                let alpha = v("_OverlayAlpha");
                bar = Some(BarValues {
                    overlay1_st: v("_Overlay1_ST"),
                    overlay2_st: v("_Overlay2_ST"),
                    bar: [f("_Width"), f("_Value"), f("_EdgeWidth"), f("_BorderWidth")],
                    border_color: linear(v("_BorderColor")).to_array(),
                    overlay: [shift[0], shift[1], alpha[0], alpha[1]],
                    flags: bar_flags,
                    overlay_texture: m.textures.get("_OverlayTex").copied().unwrap_or(0),
                });
                *census.drawn.entry(m.shader.clone()).or_default() += 1;
            }
            "uGUI/Compass" => {
                flags |= HARD_CLIP | COMPASS;
                *census.drawn.entry(m.shader.clone()).or_default() += 1;
            }
            // `UI/Circle` (M9u8b2b): one pass, `SrcAlpha OneMinusSrcAlpha`.
            "UI/Circle" => {
                flags |= HARD_CLIP | CIRCLE;
                *census.drawn.entry(m.shader.clone()).or_default() += 1;
            }
            // `UI/Icon` (M9u7c1): its variant without keywords is
            // `UI/Simple`'s program without premultiplying, blending by
            // `_SrcFactor`/`_DstFactor` for colour and alpha; with
            // `SLICE_9_GRID` (M9u8b2b) the UV first goes round the border.
            sn_ui::icon::SHADER
                if !ICON_KEYWORDS
                    .iter()
                    .any(|k| *k != "SLICE_9_GRID" && *k != "NOTIFICATION" && m.has_keyword(k)) =>
            {
                flags |= HARD_CLIP;
                let mut key = m.shader.clone();
                if m.has_keyword("NOTIFICATION") {
                    // M9u8b5b: the glow (blobs 144 and 176).
                    flags |= NOTIFICATION;
                    key.push_str(" NOTIFICATION");
                    let size = m
                        .colors
                        .get("_Size")
                        .copied()
                        .unwrap_or([100.0, 100.0, 0.0, 0.0]);
                    notification = [
                        m.float("_NotificationStrength"),
                        m.float("_PDATime"),
                        size[0],
                        size[1],
                    ];
                    overlay = m
                        .textures
                        .get("_NotificationOverlayTex")
                        .copied()
                        .unwrap_or(0);
                }
                if m.has_keyword("SLICE_9_GRID") {
                    flags |= SLICE_9;
                    key.push_str(" SLICE_9_GRID");
                }
                let f = |k: &str, default: f32| {
                    if m.has(k) { m.float(k) } else { default }
                };
                let (src, dst) = (f("_SrcFactor", 5.0) as u8, f("_DstFactor", 10.0) as u8);
                blend = [src, dst, src, dst];
                *census.drawn.entry(key).or_default() += 1;
            }
            "UI/Simple" => {
                flags |= HARD_CLIP;
                if m.has_keyword("ALPHA_PREMULTIPLY") {
                    flags |= PREMULTIPLY;
                }
                let f = |k: &str, default: f32| {
                    if m.has(k) { m.float(k) } else { default }
                };
                blend = [
                    f("_SrcFactor", 5.0) as u8,
                    f("_DstFactor", 10.0) as u8,
                    f("_SrcFactorA", 5.0) as u8,
                    f("_DstFactorA", 10.0) as u8,
                ];
                *census.drawn.entry(m.shader.clone()).or_default() += 1;
            }
            other => {
                flags |= MAIN_TEX_ST;
                if clip.is_some() {
                    flags |= SOFT_CLIP;
                }
                if other == "UI/Default" {
                    *census.drawn.entry(m.shader.clone()).or_default() += 1;
                } else {
                    *census
                        .as_default
                        .entry(format!("{other} ({})", m.name))
                        .or_default() += 1;
                }
            }
        }
        let st = if flags & COMPASS != 0 {
            [
                m.float("_Angle"),
                m.float("_AlphaFrom"),
                m.float("_AlphaTo"),
                0.0,
            ]
        } else if flags & SLICE_9 != 0 {
            let size = m
                .colors
                .get("_Size")
                .copied()
                .unwrap_or([100.0, 100.0, 0.0, 0.0]);
            [m.float("_Radius"), size[0], size[1], 0.0]
        } else if flags & CIRCLE != 0 {
            [m.float("_TileX"), m.float("_TileY"), m.float("_Size"), 0.0]
        } else {
            m.colors
                .get("_MainTex_ST")
                .copied()
                .unwrap_or([1.0, 1.0, 0.0, 0.0])
        };
        let color = linear(m.colors.get("_Color").copied().unwrap_or([1.0; 4])).to_array();
        let base = (vertices.len() / VERTEX_BYTES) as u32;
        for (v, z) in it.vertices.iter().zip(&it.z) {
            for f in [v.pos[0], v.pos[1], *z] {
                vertices.extend_from_slice(&f.to_le_bytes());
            }
            vertices.extend_from_slice(&v.color);
            for f in [v.uv[0], v.uv[1], v.uv1[0], v.uv1[1]] {
                vertices.extend_from_slice(&f.to_le_bytes());
            }
        }
        let first_index = indices.len() as u32;
        indices.extend(it.indices.iter().map(|i| i + base));
        draws.push(UiDrawCmd {
            first_index,
            index_count: it.indices.len() as u32,
            texture: it.texture,
            root: it.root,
            blend,
            color,
            st,
            clip: clip.unwrap_or(NO_CLIP),
            softness: [it.softness[0] as f32, it.softness[1] as f32],
            flags,
            notification,
            overlay,
            text,
            bar,
        });
        // Draws that cover their whole canvas (a full-screen panel hides
        // everything behind it): logged with their colour.
        if let Some(p) = placements.get(&it.root).filter(|_| log) {
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for v in &it.vertices {
                for k in 0..2 {
                    lo[k] = lo[k].min(v.pos[k]);
                    hi[k] = hi[k].max(v.pos[k]);
                }
            }
            let area = (hi[0] - lo[0]) * (hi[1] - lo[1]) / (p.size[0] * p.size[1]);
            let alpha = it.vertices.iter().map(|v| v.color[3]).max().unwrap_or(0);
            if area >= 0.05 && alpha > 0 {
                info!(
                    "game UI: large draw {} ({}): {:.0} % of the canvas, max alpha {alpha}",
                    tree.nodes[it.node].path,
                    it.kind,
                    area * 100.0
                );
            }
            if hi[0] - lo[0] >= p.size[0] * 0.95 && hi[1] - lo[1] >= p.size[1] * 0.95 {
                let alpha = it.vertices.iter().map(|v| v.color[3]).min().unwrap_or(0);
                info!(
                    "game UI: full-canvas draw {} ({}, {}): colour {:?}, min alpha {alpha}",
                    tree.nodes[it.node].path,
                    it.kind,
                    m.shader,
                    it.vertices.first().map(|v| v.color)
                );
            }
        }
        let g = groups
            .entry(tree.nodes[it.canvas].path.clone())
            .or_default();
        g.0 += 1;
        g.1 += it.vertices.len();
        g.2.insert(it.texture);
    }
    // The shaders drawn and the fallbacks, logged again when either set
    // changes (M9u8b2b: the PDA's draws come later than the first build).
    let sets = (
        census.drawn.keys().cloned().collect::<Vec<_>>(),
        census.as_default.keys().cloned().collect::<Vec<_>>(),
    );
    if !log && seen.as_ref() != Some(&sets) {
        info!(
            "game UI census changed: by shader {:?}; drawn as UI/Default (not ported yet) {:?}",
            census.drawn, census.as_default
        );
    }
    *seen = Some(sets);
    if log {
        info!(
            "game UI {}×{}: {} draws, {} vertices, {} indices; by shader {:?}; drawn as UI/Default (not ported yet) {:?}; texts {} (TextMeshPro/Distance Field); skipped: {} DepthClear (depth only), {} without a placed canvas",
            screen[0],
            screen[1],
            draws.len(),
            vertices.len() / VERTEX_BYTES,
            indices.len(),
            census.drawn,
            census.as_default,
            census.texts,
            census.depth_clear,
            census.without_placement
        );
        for (canvas, (n, v, t)) in &groups {
            info!(
                "game UI canvas {canvas}: {n} draw calls, {v} vertices, {} textures",
                t.len()
            );
        }
    }
    UiBuild {
        screen,
        field_of_view,
        vertices,
        indices,
        draws,
        placements,
        pda_root: None,
        textures: Vec::new(),
        seconds: 0.0,
        resized: log,
        item_clicks: Vec::new(),
        item_moves: Vec::new(),
        tooltip: None,
    }
}

// ---- The main world ----

fn request_build(mut worker: ResMut<UiWorker>, windows: Query<&Window, With<PrimaryWindow>>) {
    let Ok(window) = windows.single() else {
        return;
    };
    let size = [
        window.physical_width() as i32,
        window.physical_height() as i32,
    ];
    if size[0] <= 0 || size[1] <= 0 || worker.requested == Some(size) {
        return;
    }
    worker.requested = Some(size);
    let _ = worker.requests.send(Request::Size(size));
}

/// Each frame, what the HUD's rules read (M9u6b1): the player's oxygen and
/// health (`crate::player::Hud`, filled by the player), `Time`'s step and
/// clock, whether the PDA is open (M9u6b2: the icons turn), the depth
/// (M9u6c1). The worker adds `Survival`'s food and water. Our player is
/// never healed and never eats or drinks yet: no punch events.
#[allow(clippy::too_many_arguments)]
fn send_inputs(
    worker: Res<UiWorker>,
    hud: Res<crate::player::Hud>,
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut panel: Option<ResMut<crate::inventory::Panel>>,
    session: Option<Res<crate::session::Session>>,
    sim: Option<Res<crate::player::PlayerSim>>,
    text: Option<Res<crate::items::Text>>,
    camera: Query<&Transform, With<GameUiCamera>>,
    frame: Res<GameUiFrame>,
    mut compass_tech: Local<Option<Option<i32>>>,
) {
    let _perf = crate::perf::span("game_ui::send_inputs");
    // Our player's containers for the tab's icons (M9u8b2a).
    let tab_contents = match &session {
        Some(s) => {
            let me = s.player();
            s.with_items(|items, _| {
                let p = items.players.get(&me?)?;
                let techs = &items.rules().techs;
                let eq = &p.inventory.equipment;
                let compatible = |tech: i32| -> Vec<String> {
                    let t = techs.equipment_type(tech);
                    eq.slots()
                        .filter(|(s, _)| eq.table().is_compatible(t, eq.table().slot_type(s)))
                        .map(|(s, _)| s.to_string())
                        .collect()
                };
                let storage = panel
                    .as_ref()
                    .filter(|p| p.with_locker())
                    .map(|_| &items.locker);
                let place = sn_sim::inventory::DropPlace {
                    in_escape_pod: sim.as_ref().and_then(|s| s.in_pod()).unwrap_or(false),
                    ..Default::default()
                };
                let metadata = |id: sn_sim::inventory::ItemId, tech, in_grid| {
                    use sn_sim::inventory::{Button, item_action, item_actions};
                    let class = u32::try_from(id.0)
                        .ok()
                        .and_then(|id| items.classes.get(&sn_protocol::ItemId(id)))
                        .cloned();
                    let eatable = class
                        .as_ref()
                        .and_then(|c| items.rules().pickups.get(c))
                        .is_some_and(|i| i.eatable);
                    let all = item_actions(techs, &p.inventory, storage, id, eatable, place);
                    (
                        [
                            item_action(all, Button::Left),
                            item_action(all, Button::Right),
                        ],
                        in_grid && techs.equipment_type(tech) == 1,
                        class,
                    ) // EquipmentType.Hand
                };
                let entries = |c: &sn_sim::inventory::Container| -> Vec<TabEntry> {
                    let (placed, _) = c.layout();
                    placed
                        .iter()
                        .filter_map(|q| {
                            let item = c.get(q.id)?;
                            let tech = item.tech;
                            let (actions, bind, class) =
                                metadata(q.id, tech, std::ptr::eq(c, &p.inventory.container));
                            Some(TabEntry {
                                id: q.id.0,
                                tech,
                                at: (q.x, q.y),
                                size: (q.width, q.height),
                                compatible: compatible(tech),
                                actions,
                                bind,
                                class,
                                charge: item.charge,
                            })
                        })
                        .collect()
                };
                let equipment = p
                    .inventory
                    .equipment
                    .slots()
                    .filter_map(|(slot, item)| {
                        let i = item?;
                        let (actions, bind, class) = metadata(i.id, i.tech, false);
                        Some((
                            slot.to_string(),
                            TabEntry {
                                id: i.id.0,
                                tech: i.tech,
                                at: (0, 0),
                                size: techs.size(i.tech),
                                compatible: compatible(i.tech),
                                actions,
                                bind,
                                class,
                                charge: i.charge,
                            },
                        ))
                    })
                    .collect();
                Some(TabContents {
                    stores: Some(drag::Stores {
                        inventory: p.inventory.clone(),
                        locker: items.locker.clone(),
                        techs: techs.clone(),
                    }),
                    inventory: entries(&p.inventory.container),
                    locker: entries(&items.locker),
                    equipment,
                })
            })
            .flatten()
            .unwrap_or_default()
        }
        None => TabContents::default(),
    };
    let pda_clicks = PdaClicks {
        clicks: panel
            .as_mut()
            .map(|p| std::mem::take(&mut p.screen_clicks))
            .unwrap_or_default(),
        pose: frame.pda_pose,
        pointer: panel.as_ref().and_then(|p| p.pointer),
        hover: panel.as_ref().and_then(|p| p.hover),
        drag: panel
            .as_mut()
            .map(|p| std::mem::take(&mut p.drag_inputs))
            .unwrap_or_default(),
    };
    // `TechType.Compass`, by its name in the game's code.
    let compass_tech = *compass_tech.get_or_insert_with(|| {
        text.as_ref().and_then(|t| {
            t.0.tech_names
                .iter()
                .find(|(_, n)| n.as_str() == "Compass")
                .map(|(v, _)| *v)
        })
    });
    // `Inventory.main.equipment.GetCount(TechType.Compass) > 0`.
    // `Inventory.Pickup`'s `uGUI_IconNotifier.Play(techType, From)`
    // (M9u7c2): this frame's accepted pick-ups of our own player, by the
    // tech type's name (`TechType.AsString`).
    let plays: Vec<String> = match (&session, &text) {
        (Some(s), Some(t)) => {
            let me = s.player();
            s.due
                .iter()
                .filter_map(|e| match &e.kind {
                    sn_protocol::WorldEventKind::Item {
                        player,
                        event: sn_protocol::ItemEvent::PickedUp { class, .. },
                        ..
                    } if Some(*player) == me => s
                        .with_items(|items, _| items.rules().pickups.get(class).map(|i| i.tech))
                        .flatten(),
                    _ => None,
                })
                .map(|tech| {
                    t.0.tech_names
                        .get(&tech)
                        .cloned()
                        .unwrap_or_else(|| tech.to_string())
                })
                .collect()
        }
        _ => Vec::new(),
    };
    let compass = match (&session, compass_tech) {
        (Some(s), Some(tech)) => {
            let me = s.player();
            s.with_items(|items, _| {
                items
                    .players
                    .get(&me?)
                    .map(|p| p.inventory.equipment.items().any(|i| i.tech == tech))
            })
            .flatten()
            .unwrap_or(false)
        }
        _ => false,
    };
    let _ = worker.requests.send(Request::Frame(Box::new((
        HudInputs {
            active: hud.shown,
            mode: hud_mode(false, false, false),
            oxygen: hud.oxygen as f32,
            oxygen_capacity: hud.capacity as f32,
            health: hud.health as f32,
            max_health: hud.max_health as f32,
            food: 0.0,
            water: 0.0,
            delta_time: time.delta_secs(),
            time: time.elapsed_secs(),
            pda_open: panel.as_ref().is_some_and(|p| p.pda.in_use()),
            healed: None,
            ate: None,
            drank: None,
            depth: hud.depth as f32,
            compass,
            yaw: camera.single().map_or(0.0, |t| euler_y(t.rotation)),
        },
        HandFrame {
            hover: hud.hand.clone(),
        },
        DeathInputs {
            delta_time: time.delta_secs(),
            unscaled_delta_time: real.delta_secs(),
            time: time.elapsed_secs(),
            // `Player.Update` runs while the player exists.
            suffocation: hud.shown.then_some(hud.overlay as f32),
            killed: hud.killed,
            vignette: hud.vignette,
            respawn_show: hud.respawn_show,
            respawn_hide: hud.respawn_hide,
        },
        MessageKeys {
            early: hud.messages_early.clone(),
            late: hud.messages_late.clone(),
        },
        plays,
        panel.as_ref().map_or(PdaFlags::default(), |p| PdaFlags {
            using: p.pda.using_pda(),
            focused: p.pda.focused(),
            active: p.pda.active(),
            with_locker: p.with_locker(),
        }),
        pda_clicks,
        tab_contents,
    ))));
}

fn receive_build(
    worker: Res<UiWorker>,
    mut frame: ResMut<GameUiFrame>,
    mut images: ResMut<Assets<Image>>,
) {
    let _perf = crate::perf::span("game_ui::receive_build");
    let Ok(rx) = worker.builds.lock() else {
        return;
    };
    // The latest build's draws; every build's textures (the worker sends
    // each texture once, so a build skipped here must not lose them) and
    // its resize.
    let mut latest: Option<UiBuild> = None;
    while let Ok(b) = rx.try_recv() {
        match b {
            Ok(mut b) => {
                // Every build's item clicks.
                frame.item_clicks.append(&mut b.item_clicks);
                frame.item_moves.append(&mut b.item_moves);
                if let Some(old) = latest.take() {
                    let mut textures = old.textures;
                    textures.append(&mut b.textures);
                    b.textures = textures;
                    b.resized |= old.resized;
                }
                latest = Some(b);
            }
            Err(e) => error!("game UI: {e}"),
        }
    }
    let Some(mut build) = latest else {
        return;
    };
    let mut map = (*frame.images).clone();
    for t in build.textures.drain(..) {
        crate::perf::count("game_ui::image_bytes", t.data.len());
        let format = match t.format {
            TexelFormat::RgbaSrgb => TextureFormat::Rgba8UnormSrgb,
            TexelFormat::Rgba => TextureFormat::Rgba8Unorm,
            TexelFormat::R8 => TextureFormat::R8Unorm,
        };
        let mut image = Image::new(
            Extent3d {
                width: t.width,
                height: t.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            t.data,
            format,
            RenderAssetUsages::RENDER_WORLD,
        );
        let mut descriptor = if t.point {
            ImageSamplerDescriptor::nearest()
        } else {
            ImageSamplerDescriptor::linear()
        };
        let mode = if t.repeat {
            ImageAddressMode::Repeat
        } else {
            ImageAddressMode::ClampToEdge
        };
        descriptor.set_address_mode(mode);
        descriptor.mipmap_filter = ImageFilterMode::Nearest;
        image.sampler = ImageSampler::Descriptor(descriptor);
        // A font atlas that grew replaces its image in place.
        match map.get(&t.key) {
            Some((handle, _)) => {
                let _ = images.insert(handle.id(), image);
            }
            None => {
                map.insert(t.key, (images.add(image), t.alpha_only));
            }
        }
    }
    if build.resized {
        info!(
            "game UI: built for {}×{} in {:.3} s; {} textures on the GPU",
            build.screen[0],
            build.screen[1],
            build.seconds,
            map.len()
        );
    }
    frame.images = Arc::new(map);
    frame.build = Some(Arc::new(build));
    frame.generation += 1;
}

/// The PDA screen canvas's pose (M9u8a4a): `uGUI_CanvasScaler.UpdateTransform`
/// in Inversed mode, `Inverse(cam.rotation) × (anchor − cam)` and
/// `Inverse(cam.rotation) × anchor.rotation`, from this frame's final
/// transforms, in Unity's axes (z mirrored). Logged each second while the
/// PDA is active.
fn pda_pose(
    time: Res<Time>,
    mut frame: ResMut<GameUiFrame>,
    drive: Option<Res<crate::body::BodyDrive>>,
    rigs: Query<&crate::objects::NamedNodes, With<crate::objects::PlayerBodyRig>>,
    globals: Query<&GlobalTransform>,
    camera: Query<&GlobalTransform, With<GameUiCamera>>,
    mut logged: Local<i64>,
) {
    let anchor = rigs
        .iter()
        .flat_map(|n| n.0.iter())
        .find(|(name, _)| name == "ScreenAnchor")
        .and_then(|(_, e)| globals.get(*e).ok());
    let pose = match (
        drive.as_ref().is_some_and(|d| d.pda_model),
        anchor,
        camera.single(),
    ) {
        (true, Some(a), Ok(c)) => {
            let rel = c.affine().inverse() * a.affine();
            let (_, r, t) = rel.to_scale_rotation_translation();
            Some(([t.x, t.y, -t.z], [-r.x, -r.y, r.z, r.w]))
        }
        _ => None,
    };
    if let Some((p, r)) = pose {
        let second = time.elapsed_secs().floor() as i64;
        if second != *logged {
            *logged = second;
            // The canvas's +z against the camera's.
            let q = Quat::from_xyzw(r[0], r[1], r[2], r[3]);
            let angle = (q * Vec3::Z).angle_between(Vec3::Z).to_degrees();
            info!(
                "game UI PDA screen pose at {:.3} s: ({:.4}, {:.4}, {:.4}) m in the camera's space, its +z {angle:.2}° from the camera's",
                time.elapsed_secs(),
                p[0],
                p[1],
                p[2]
            );
        }
    }
    if frame.pda_pose != pose {
        frame.pda_pose = pose;
    }
}

// ---- The render world ----

/// One item's values; must match `Item` in `game_ui.wgsl`.
#[derive(Clone, Copy, Default, ShaderType)]
struct GpuItem {
    clip_from_canvas: Mat4,
    color: Vec4,
    st: Vec4,
    texture_add: Vec4,
    clip_rect: Vec4,
    softness: Vec4,
    pixel: Vec4,
    notification: Vec4,
}

#[derive(Clone, Default, ShaderType)]
struct GpuItems {
    #[shader(size(runtime))]
    items: Vec<GpuItem>,
}

/// One text item's values; must match `TextItem` in `game_ui_text.wgsl`.
#[derive(Clone, Copy, Default, ShaderType)]
struct GpuTextItem {
    clip_from_canvas: Mat4,
    world_from_canvas: Mat4,
    face_color: Vec4,
    outline_color: Vec4,
    underlay_color: Vec4,
    face: Vec4,
    weight: Vec4,
    scale: Vec4,
    underlay: Vec4,
    atlas: Vec4,
    offset: Vec4,
    clip_rect: Vec4,
    normal: Vec4,
    pixel: Vec4,
}

#[derive(Clone, Default, ShaderType)]
struct GpuTextItems {
    #[shader(size(runtime))]
    items: Vec<GpuTextItem>,
}

/// One bar item's values; must match `BarItem` in `game_ui_bar.wgsl`.
#[derive(Clone, Copy, Default, ShaderType)]
struct GpuBarItem {
    clip_from_canvas: Mat4,
    color: Vec4,
    overlay1_st: Vec4,
    overlay2_st: Vec4,
    bar: Vec4,
    border_color: Vec4,
    overlay: Vec4,
    texture_add: Vec4,
    clip_rect: Vec4,
    flags: Vec4,
}

#[derive(Clone, Default, ShaderType)]
struct GpuBarItems {
    #[shader(size(runtime))]
    items: Vec<GpuBarItem>,
}

#[derive(Resource)]
struct UiPipelines {
    items_layout: BindGroupLayoutDescriptor,
    text_items_layout: BindGroupLayoutDescriptor,
    bar_items_layout: BindGroupLayoutDescriptor,
    texture_layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
    text_shader: Handle<Shader>,
    bar_shader: Handle<Shader>,
    /// By target format, blend and kind.
    by_key: HashMap<(TextureFormat, BlendKey, Kind), CachedRenderPipelineId>,
}

/// This frame's GPU data.
#[derive(Resource, Default)]
struct GpuUi {
    generation: u64,
    vertices: Option<Buffer>,
    indices: Option<Buffer>,
    /// The values the items were made for: screen, generation.
    made_for: Option<([u32; 2], u64)>,
    items: StorageBuffer<GpuItems>,
    text_items: StorageBuffer<GpuTextItems>,
    bar_items: StorageBuffer<GpuBarItems>,
    /// Each draw's instance in its kind's buffer.
    instances: Vec<u32>,
    /// CPU time of preparing, summed over the frames since the last log.
    cpu: (f64, u32),
}

fn init_gpu(mut commands: Commands, asset_server: Res<AssetServer>) {
    let items_layout = BindGroupLayoutDescriptor::new(
        "game_ui_items_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (storage_buffer_read_only::<GpuItems>(false),),
        ),
    );
    let text_items_layout = BindGroupLayoutDescriptor::new(
        "game_ui_text_items_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (storage_buffer_read_only::<GpuTextItems>(false),),
        ),
    );
    let bar_items_layout = BindGroupLayoutDescriptor::new(
        "game_ui_bar_items_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (storage_buffer_read_only::<GpuBarItems>(false),),
        ),
    );
    let texture_layout = BindGroupLayoutDescriptor::new(
        "game_ui_texture_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    commands.insert_resource(UiPipelines {
        items_layout,
        text_items_layout,
        bar_items_layout,
        texture_layout,
        shader: asset_server.load("embedded://sn_client/game_ui.wgsl"),
        text_shader: asset_server.load("embedded://sn_client/game_ui_text.wgsl"),
        bar_shader: asset_server.load("embedded://sn_client/game_ui_bar.wgsl"),
        by_key: HashMap::new(),
    });
}

/// The pipelines each view needs: one per blend state and kind (image,
/// text, bar) in the build.
#[derive(Component)]
struct UiPipelineIds(HashMap<(BlendKey, Kind), CachedRenderPipelineId>);

fn pipeline_descriptor(
    p: &UiPipelines,
    format: TextureFormat,
    blend: BlendKey,
    kind: Kind,
) -> RenderPipelineDescriptor {
    let (items_layout, shader) = match kind {
        Kind::Text => (&p.text_items_layout, &p.text_shader),
        Kind::Bar => (&p.bar_items_layout, &p.bar_shader),
        Kind::Image => (&p.items_layout, &p.shader),
    };
    // A bar also samples its overlay texture (group 2), an image the
    // notification's (M9u8b5b).
    let mut layout = vec![items_layout.clone(), p.texture_layout.clone()];
    if kind != Kind::Text {
        layout.push(p.texture_layout.clone());
    }
    let attribute = |format, offset, shader_location| VertexAttribute {
        format,
        offset,
        shader_location,
    };
    let f = |v: u8| blend_factor(f32::from(v));
    let state = BlendState {
        color: BlendComponent {
            src_factor: f(blend[0]),
            dst_factor: f(blend[1]),
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent {
            src_factor: f(blend[2]),
            dst_factor: f(blend[3]),
            operation: BlendOperation::Add,
        },
    };
    RenderPipelineDescriptor {
        label: Some("game_ui_pipeline".into()),
        layout,
        vertex: VertexState {
            shader: shader.clone(),
            shader_defs: Vec::new(),
            entry_point: Some("vertex".into()),
            buffers: vec![VertexBufferLayout {
                array_stride: VERTEX_BYTES as u64,
                step_mode: VertexStepMode::Vertex,
                attributes: vec![
                    attribute(VertexFormat::Float32x3, 0, 0),
                    attribute(VertexFormat::Unorm8x4, 12, 1),
                    attribute(VertexFormat::Float32x2, 16, 2),
                    attribute(VertexFormat::Float32x2, 24, 3),
                ],
            }],
        },
        // `Cull Off` (the in-scope materials; `UI/Simple`'s `_Cull` is
        // logged with the materials, not applied).
        primitive: PrimitiveState {
            cull_mode: None,
            ..default()
        },
        depth_stencil: None,
        fragment: Some(FragmentState {
            shader: shader.clone(),
            shader_defs: Vec::new(),
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState {
                format,
                blend: Some(state),
                write_mask: ColorWrites::ALL,
            })],
        }),
        ..default()
    }
}

fn prepare_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut pipelines: ResMut<UiPipelines>,
    frame: Option<Res<GameUiFrame>>,
    views: Query<(Entity, &ExtractedView), With<GameUiCamera>>,
) {
    let Some(build) = frame.as_ref().and_then(|f| f.build.clone()) else {
        return;
    };
    let mut blends: Vec<(BlendKey, Kind)> =
        build.draws.iter().map(|d| (d.blend, d.kind())).collect();
    blends.sort();
    blends.dedup();
    for (entity, view) in &views {
        let mut ids = HashMap::new();
        for b in &blends {
            let key = (view.target_format, b.0, b.1);
            let id = match pipelines.by_key.get(&key) {
                Some(id) => *id,
                None => {
                    let d = pipeline_descriptor(&pipelines, view.target_format, b.0, b.1);
                    let id = pipeline_cache.queue_render_pipeline(d);
                    pipelines.by_key.insert(key, id);
                    id
                }
            };
            ids.insert(*b, id);
        }
        commands.entity(entity).insert(UiPipelineIds(ids));
    }
}

/// The clip-space matrix of a canvas placed in front of the UI camera
/// (at the origin, Unity's axes: +z forward), for a screen's aspect: the
/// game's perspective, depth halfway (our pass has no depth buffer).
pub(crate) fn clip_from_canvas(p: &Placement, field_of_view: f32, aspect: f32) -> Mat4 {
    clip_from_world(field_of_view, aspect) * world_from_canvas(p)
}

/// A placed canvas's transform to the world (Unity's axes).
fn world_from_canvas(p: &Placement) -> Mat4 {
    let w = p.world_from_canvas();
    Mat4::from_cols(
        Vec4::new(w[0][0], w[1][0], w[2][0], 0.0),
        Vec4::new(w[0][1], w[1][1], w[2][1], 0.0),
        Vec4::new(w[0][2], w[1][2], w[2][2], 0.0),
        Vec4::new(w[0][3], w[1][3], w[2][3], 1.0),
    )
}

/// TMP's mesh normal (0, 0, -1) in the world: `mul(normal,
/// (float3x3)unity_WorldToObject)`, normalized.
fn text_normal(world_from_canvas: &Mat4) -> Vec3 {
    let m = Mat3::from_mat4(*world_from_canvas);
    (m.inverse().transpose() * Vec3::NEG_Z).normalize_or_zero()
}

/// The UI camera's projection (at the origin, no rotation).
fn clip_from_world(field_of_view: f32, aspect: f32) -> Mat4 {
    let t = (field_of_view.to_radians() * 0.5).tan();
    Mat4::from_cols(
        Vec4::new(1.0 / (t * aspect), 0.0, 0.0, 0.0),
        Vec4::new(0.0, 1.0 / t, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.5, 1.0),
        Vec4::ZERO,
    )
}

fn prepare_buffers(
    mut gpu: ResMut<GpuUi>,
    frame: Option<Res<GameUiFrame>>,
    views: Query<&ExtractedView, With<GameUiCamera>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let _perf = crate::perf::span("game_ui::prepare_buffers");
    let begin = Instant::now();
    let Some(frame) = frame else {
        return;
    };
    let Some(build) = frame.build.clone() else {
        return;
    };
    if gpu.generation != frame.generation || gpu.vertices.is_none() {
        gpu.generation = frame.generation;
        gpu.vertices = (!build.vertices.is_empty()).then(|| {
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("game_ui_vertices"),
                contents: &build.vertices,
                usage: BufferUsages::VERTEX,
            })
        });
        let index_bytes: Vec<u8> = build.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        gpu.indices = (!index_bytes.is_empty()).then(|| {
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("game_ui_indices"),
                contents: &index_bytes,
                usage: BufferUsages::INDEX,
            })
        });
    }
    let Some(view) = views.iter().next() else {
        return;
    };
    let screen = [view.viewport.z, view.viewport.w];
    // The PDA's screen moves every frame without a new build.
    if gpu.made_for == Some((screen, frame.generation)) && frame.pda_pose.is_none() {
        return;
    }
    gpu.made_for = Some((screen, frame.generation));
    let aspect = screen[0] as f32 / screen[1].max(1) as f32;
    let projection = clip_from_world(build.field_of_view, aspect);
    let mut items = Vec::with_capacity(build.draws.len());
    let mut text_items = Vec::new();
    let mut bar_items = Vec::new();
    let mut instances = Vec::with_capacity(build.draws.len());
    let pixel = Vec4::new(
        screen[0] as f32,
        screen[1] as f32,
        projection.x_axis.x,
        projection.y_axis.y,
    );
    let pda_placement = match (build.pda_root, frame.pda_pose) {
        (Some(root), Some((position, rotation))) => {
            build.placements.get(&root).map(|p| Placement {
                position,
                rotation,
                ..*p
            })
        }
        _ => None,
    };
    // The tooltip (M9u8b3b3) sits in the PDA canvas's plane: its pivot at
    // a point of that canvas, its scale that canvas's × k.
    let tooltip_placement = match (&build.tooltip, &pda_placement) {
        (Some(t), Some(p)) => build.placements.get(&t.root).map(|own| Placement {
            position: sn_ui::draw::apply(&p.world_from_canvas(), t.point),
            rotation: p.rotation,
            scale: [p.scale[0] * t.k, p.scale[1] * t.k, p.scale[2] * t.k],
            ..*own
        }),
        _ => None,
    };
    for d in &build.draws {
        let placed = match (&pda_placement, &tooltip_placement, &build.tooltip) {
            (Some(p), _, _) if Some(d.root) == build.pda_root => Some(p),
            (_, Some(p), Some(t)) if d.root == t.root => Some(p),
            _ => build.placements.get(&d.root),
        };
        let Some(p) = placed else {
            instances.push(u32::MAX);
            continue;
        };
        if let Some(t) = &d.text {
            instances.push(text_items.len() as u32);
            let world = world_from_canvas(p);
            text_items.push(GpuTextItem {
                clip_from_canvas: clip_from_canvas(p, build.field_of_view, aspect),
                world_from_canvas: world,
                face_color: Vec4::from_array(t.face_color),
                outline_color: Vec4::from_array(t.outline_color),
                underlay_color: Vec4::from_array(t.underlay_color),
                face: Vec4::from_array(t.face),
                weight: Vec4::from_array(t.weight),
                scale: Vec4::new(t.scale[0], t.scale[1], t.scale[2], p.scale[1].abs()),
                underlay: Vec4::from_array(t.underlay),
                atlas: Vec4::new(t.atlas[0], t.atlas[1], t.atlas[2], d.flags as f32),
                offset: Vec4::from_array(t.offset),
                clip_rect: Vec4::from_array(d.clip),
                normal: text_normal(&world).extend(0.0),
                pixel,
            });
            continue;
        }
        let alpha_only = frame.images.get(&d.texture).is_some_and(|(_, a)| *a);
        let texture_add = if alpha_only {
            Vec4::new(1.0, 1.0, 1.0, 0.0)
        } else {
            Vec4::ZERO
        };
        if let Some(b) = &d.bar {
            instances.push(bar_items.len() as u32);
            bar_items.push(GpuBarItem {
                clip_from_canvas: clip_from_canvas(p, build.field_of_view, aspect),
                color: Vec4::from_array(d.color),
                overlay1_st: Vec4::from_array(b.overlay1_st),
                overlay2_st: Vec4::from_array(b.overlay2_st),
                bar: Vec4::from_array(b.bar),
                border_color: Vec4::from_array(b.border_color),
                overlay: Vec4::from_array(b.overlay),
                texture_add,
                clip_rect: Vec4::from_array(d.clip),
                flags: Vec4::new(b.flags as f32, 0.0, 0.0, 0.0),
            });
            continue;
        }
        instances.push(items.len() as u32);
        items.push(GpuItem {
            clip_from_canvas: clip_from_canvas(p, build.field_of_view, aspect),
            color: Vec4::from_array(d.color),
            st: Vec4::from_array(d.st),
            texture_add,
            clip_rect: Vec4::from_array(d.clip),
            softness: Vec4::new(d.softness[0], d.softness[1], d.flags as f32, 0.0),
            pixel,
            notification: Vec4::from_array(d.notification),
        });
    }
    // Storage buffers may not be empty.
    if items.is_empty() {
        items.push(GpuItem::default());
    }
    if text_items.is_empty() {
        text_items.push(GpuTextItem::default());
    }
    if bar_items.is_empty() {
        bar_items.push(GpuBarItem::default());
    }
    gpu.items.set(GpuItems { items });
    gpu.items.write_buffer(&device, &queue);
    gpu.text_items.set(GpuTextItems { items: text_items });
    gpu.text_items.write_buffer(&device, &queue);
    gpu.bar_items.set(GpuBarItems { items: bar_items });
    gpu.bar_items.write_buffer(&device, &queue);
    gpu.instances = instances;
    let spent = begin.elapsed().as_secs_f64();
    gpu.cpu.0 += spent;
    gpu.cpu.1 += 1;
    if !build.resized {
        return;
    }
    info!(
        "game UI: {} items made for {}×{} in {:.3} ms",
        build.draws.len(),
        screen[0],
        screen[1],
        spent * 1000.0
    );
}

/// The pass's own CPU cost per frame (recording the draws), logged every
/// 600 frames.
#[derive(Default)]
struct PassCost {
    seconds: f64,
    frames: u32,
}

#[allow(clippy::too_many_arguments)] // a render system
fn game_ui_pass(
    view: ViewQuery<(&ViewTarget, &UiPipelineIds), With<GameUiCamera>>,
    pipeline_cache: Res<PipelineCache>,
    pipelines: Res<UiPipelines>,
    gpu: Res<GpuUi>,
    frame: Option<Res<GameUiFrame>>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    mut cost: Local<PassCost>,
    mut ctx: RenderContext,
) {
    let _perf = crate::perf::span("game_ui::game_ui_pass");
    let begin = Instant::now();
    let (target, ids) = view.into_inner();
    let Some(frame) = frame else {
        return;
    };
    let Some(build) = frame.build.clone() else {
        return;
    };
    let (Some(vertices), Some(indices), Some(items), Some(text_items), Some(bar_items)) = (
        &gpu.vertices,
        &gpu.indices,
        gpu.items.binding(),
        gpu.text_items.binding(),
        gpu.bar_items.binding(),
    ) else {
        return;
    };
    let items_group = device.create_bind_group(
        "game_ui_items",
        &pipeline_cache.get_bind_group_layout(&pipelines.items_layout),
        &BindGroupEntries::sequential((items,)),
    );
    let text_items_group = device.create_bind_group(
        "game_ui_text_items",
        &pipeline_cache.get_bind_group_layout(&pipelines.text_items_layout),
        &BindGroupEntries::sequential((text_items,)),
    );
    let bar_items_group = device.create_bind_group(
        "game_ui_bar_items",
        &pipeline_cache.get_bind_group_layout(&pipelines.bar_items_layout),
        &BindGroupEntries::sequential((bar_items,)),
    );
    let texture_layout = pipeline_cache.get_bind_group_layout(&pipelines.texture_layout);
    let mut texture_groups = HashMap::new();
    let keys = build.draws.iter().flat_map(|d| {
        [d.texture, d.overlay]
            .into_iter()
            .chain(d.bar.map(|b| b.overlay_texture))
    });
    for key in keys {
        texture_groups.entry(key).or_insert_with(|| {
            let handle = frame.images.get(&key).map_or(&frame.white, |(h, _)| h);
            images.get(handle.id()).map(|img| {
                device.create_bind_group(
                    "game_ui_texture",
                    &texture_layout,
                    &BindGroupEntries::sequential((&img.texture_view, &img.sampler)),
                )
            })
        });
    }
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("game_ui"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target.main_texture_view(),
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_vertex_buffer(0, (**vertices).slice(..));
    pass.set_index_buffer((**indices).slice(..), IndexFormat::Uint32);
    let mut current = None;
    let mut current_kind = None;
    let mut drawn = 0;
    for (k, d) in build.draws.iter().enumerate() {
        let kind = d.kind();
        let (Some(id), Some(Some(group)), Some(&i)) = (
            ids.0.get(&(d.blend, kind)),
            texture_groups.get(&d.texture),
            gpu.instances.get(k),
        ) else {
            continue;
        };
        if i == u32::MAX {
            continue;
        }
        let Some(pipeline) = pipeline_cache.get_render_pipeline(*id) else {
            continue;
        };
        if current != Some(*id) {
            pass.set_pipeline(pipeline);
            current = Some(*id);
        }
        if current_kind != Some(kind) {
            let items = match kind {
                Kind::Text => &text_items_group,
                Kind::Bar => &bar_items_group,
                Kind::Image => &items_group,
            };
            pass.set_bind_group(0, items, &[]);
            current_kind = Some(kind);
        }
        pass.set_bind_group(1, group, &[]);
        // The overlay (none: the white texture, unused without `OVERLAY`
        // or `NOTIFICATION`).
        let overlay = match (&d.bar, kind) {
            (Some(b), _) => Some(b.overlay_texture),
            (None, Kind::Image) => Some(d.overlay),
            _ => None,
        };
        if let Some(key) = overlay {
            let Some(Some(overlay)) = texture_groups.get(&key) else {
                continue;
            };
            pass.set_bind_group(2, overlay, &[]);
        }
        pass.draw_indexed(d.first_index..d.first_index + d.index_count, 0, i..i + 1);
        drawn += 1;
    }
    drop(pass);
    cost.seconds += begin.elapsed().as_secs_f64();
    cost.frames += 1;
    if cost.frames == 600 {
        info!(
            "game UI pass: {drawn} of {} draws; CPU {:.3} ms per frame (mean of 600)",
            build.draws.len(),
            cost.seconds * 1000.0 / 600.0
        );
        *cost = PassCost::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fly_camera_ui_needs_no_player_body() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<GameUiFrame>()
            .add_systems(Update, pda_pose);
        app.update();
        assert!(app.world().resource::<GameUiFrame>().pda_pose.is_none());
    }

    // The decoded formulas, as `game_ui.wgsl` computes them; the tests
    // check the formulas, not the WGSL.

    /// `UI/Default`'s soft clip factor at canvas point `p` for clip rect
    /// `r`, softness `s` and a pixel size `px`.
    fn soft_clip(p: [f32; 2], r: [f32; 4], s: [f32; 2], px: [f32; 2]) -> f32 {
        let mut out = 1.0;
        for k in 0..2 {
            let mask = p[k] * 2.0 - r[k] - r[k + 2];
            let factor = 0.25 / (0.25 * s[k] + px[k].abs());
            out *= ((r[k + 2] - r[k] - mask.abs()) * factor).clamp(0.0, 1.0);
        }
        out
    }

    #[test]
    fn euler_y_is_the_unity_yaw_whatever_the_pitch_and_roll() {
        // The player's camera: Unity yaw about +y, pitch about x
        // (`player::camera_rotation`).
        for (yaw, pitch, roll) in [
            (30.0f32, 20.0f32, 0.0f32),
            (200.0, -60.0, 0.0),
            (300.0, 10.0, 25.0),
        ] {
            let q = Quat::from_rotation_y(-yaw.to_radians())
                * Quat::from_rotation_x(-pitch.to_radians())
                * Quat::from_rotation_z(roll.to_radians());
            assert!((euler_y(q) - yaw).abs() < 1e-3, "{yaw}: {}", euler_y(q));
        }
    }

    #[test]
    fn the_soft_clip_fades_over_the_softness() {
        let r = [0.0, 0.0, 100.0, 50.0];
        // Inside, far from the edges: whole.
        assert_eq!(soft_clip([50.0, 25.0], r, [0.0, 0.0], [0.001, 0.001]), 1.0);
        // Outside: gone.
        assert_eq!(soft_clip([150.0, 25.0], r, [0.0, 0.0], [0.001, 0.001]), 0.0);
        // Softness 8 on x: the factor 1/8 per unit of 2× distance, so 2
        // units in from the edge (4 in the mask's doubled units) gives 0.5.
        let half = soft_clip([98.0, 25.0], r, [8.0, 0.0], [0.0, 0.001]);
        assert!((half - 0.5).abs() < 1e-5, "{half}");
    }

    #[test]
    fn a_canvas_corner_lands_on_the_screen_corner() {
        let s = sn_ui::scaler::ScalerSettings {
            reference_resolution: [1920.0, 1080.0],
            mode: 0,
            distance: 1.0,
            scale_mode: 1,
        };
        let cam = UiCamera::game(60.0, NEAR, FAR);
        let p = place(&s, &cam, 60.0, [2560, 1440], 1.0).unwrap();
        let m = clip_from_canvas(&p, 60.0, 2560.0 / 1440.0);
        // The canvas's top right (pivot at its centre): NDC (1, 1).
        let c = m * Vec4::new(p.size[0] / 2.0, p.size[1] / 2.0, 0.0, 1.0);
        assert!((c.x / c.w - 1.0).abs() < 1e-5 && (c.y / c.w - 1.0).abs() < 1e-5);
        // In front, with depth inside the clip volume.
        assert!(c.w > 0.0 && c.z >= 0.0 && c.z <= c.w);
        let b = m * Vec4::new(-p.size[0] / 2.0, -p.size[1] / 2.0, 0.0, 1.0);
        assert!((b.x / b.w + 1.0).abs() < 1e-5 && (b.y / b.w + 1.0).abs() < 1e-5);
    }

    /// TMP's face alpha factor (`game_ui_text.wgsl`'s fragment) for a
    /// distance `sd` (the atlas's 0–1 value, 0.5 on the outline), SDF
    /// scale, weight, outline width and softness (× ratio A already).
    fn text_alpha(sd: f32, scale: f32, weight: f32, width: f32, softness: f32) -> f32 {
        let bias = (0.5 - weight) + 0.5 / scale;
        let d = bias - sd;
        let w = width * scale;
        let s = softness * scale;
        1.0 - ((d * scale - w * 0.5 + s * 0.5) / (1.0 + s)).clamp(0.0, 1.0)
    }

    #[test]
    fn text_alpha_crosses_half_at_the_outline() {
        // Plain text: half at the glyph's edge, whole inside, none outside.
        assert!((text_alpha(0.5, 8.0, 0.0, 0.0, 0.0) - 0.5).abs() < 1e-6);
        assert_eq!(text_alpha(0.9, 8.0, 0.0, 0.0, 0.0), 1.0);
        assert_eq!(text_alpha(0.1, 8.0, 0.0, 0.0, 0.0), 0.0);
        // Weight (bold, dilate) moves the edge outwards.
        assert!(text_alpha(0.45, 8.0, 0.1, 0.0, 0.0) > 0.5);
        // An outline reaches further out.
        assert!(text_alpha(0.45, 8.0, 0.0, 0.2, 0.0) > text_alpha(0.45, 8.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn a_canvas_facing_the_camera_has_its_normal_towards_it() {
        let s = sn_ui::scaler::ScalerSettings {
            reference_resolution: [1920.0, 1080.0],
            mode: 0,
            distance: 1.0,
            scale_mode: 1,
        };
        let cam = UiCamera::game(60.0, NEAR, FAR);
        let p = place(&s, &cam, 60.0, [1920, 1080], 1.0).unwrap();
        let n = text_normal(&world_from_canvas(&p));
        // TMP's (0, 0, −1) stays −z (towards the camera at the origin),
        // unit length despite the canvas's tiny scale.
        assert!((n - Vec3::NEG_Z).length() < 1e-5, "{n:?}");
    }

    #[test]
    fn unity_blend_modes_map_to_wgpu() {
        assert_eq!(blend_factor(5.0), BlendFactor::SrcAlpha);
        assert_eq!(blend_factor(10.0), BlendFactor::OneMinusSrcAlpha);
        assert_eq!(blend_factor(1.0), BlendFactor::One);
        assert_eq!(blend_factor(0.0), BlendFactor::Zero);
    }
}
