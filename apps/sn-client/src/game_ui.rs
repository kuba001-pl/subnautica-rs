//! The game's UI drawn as the game draws it (M9u5c; `docs/DESIGN.md` M9u
//! plan, `docs/formats/ui.md`). `--game-ui` turns it on; until M9u6 removes
//! our own HUD (`hud.rs`) it is off by default, so the two never draw
//! together.
//!
//! - A worker thread reads the install once and builds `uGUI`'s draw list
//!   (`sn_assets::Assets::ui_draw_placed`) with its canvases placed for the
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
//! Shaders: `UI/Default` and `UI/Simple` (`game_ui.wgsl`), and TMP's
//! `TextMeshPro/Distance Field` (`game_ui_text.wgsl`, M9u5d) with the
//! font atlases the worker fills (sent again when glyphs are added). `uGUI/DepthClear` writes only depth,
//! which our pass does not use: skipped (counted). Other shaders (`UI/Bar`,
//! `uGUI/WorldCursor`, `UI/Circle`) are drawn as `UI/Default` and named in
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
use sn_ui::scaler::{DEFAULT_UI_SCALE, Placement, UiCamera, place};

use crate::textures::linear;

/// The main camera's planes in the main scene; the UI camera takes them
/// (`SNCameraRoot.Awake`; `docs/formats/ui.md` § Canvas placement).
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
/// Bytes per vertex: position (3 × f32), colour (4 × u8), uv, uv1.
const VERTEX_BYTES: usize = 32;

/// Item flags (`game_ui.wgsl`).
const SOFT_CLIP: u32 = 1;
const HARD_CLIP: u32 = 2;
const PREMULTIPLY: u32 = 4;
const MAIN_TEX_ST: u32 = 8;

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
    /// A TMP text's material values (`game_ui_text.wgsl`); `None` for an
    /// image.
    pub text: Option<TextValues>,
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
    /// Textures not sent before.
    pub textures: Vec<UiTexture>,
    pub seconds: f64,
}

struct Request {
    screen: [i32; 2],
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
    images: Arc<HashMap<u64, (Handle<Image>, bool)>>,
    white: Handle<Image>,
}

/// Marks the camera the UI is drawn over.
#[derive(Component, Clone, Copy, ExtractComponent)]
pub(crate) struct GameUiCamera;

pub(crate) struct GameUiPlugin {
    pub game_dir: Option<PathBuf>,
    /// The main camera's field of view (degrees), which the UI camera
    /// takes.
    pub field_of_view: f32,
}

impl Plugin for GameUiPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "game_ui.wgsl");
        bevy::asset::embedded_asset!(app, "game_ui_text.wgsl");
        let (requests, worker_requests) = channel::<Request>();
        let (worker_builds, builds) = channel();
        let game_dir = self.game_dir.clone();
        let fov = self.field_of_view;
        std::thread::Builder::new()
            .name("game-ui".into())
            .spawn(move || worker(game_dir, fov, worker_requests, worker_builds))
            .expect("spawn the game UI worker");
        app.insert_resource(UiWorker {
            requests,
            builds: Mutex::new(builds),
            requested: None,
        })
        .init_resource::<GameUiFrame>()
        .add_plugins((
            ExtractResourcePlugin::<GameUiFrame>::default(),
            ExtractComponentPlugin::<GameUiCamera>::default(),
        ))
        .add_systems(Startup, make_white)
        .add_systems(PostStartup, mark_camera)
        .add_systems(Update, (request_build, receive_build).chain());
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
    requests: Receiver<Request>,
    builds: Sender<Result<UiBuild, String>>,
) {
    if let Err(e) = run_worker(game_dir, fov, &requests, &builds) {
        let _ = builds.send(Err(e));
    }
}

fn run_worker(
    game_dir: Option<PathBuf>,
    fov: f32,
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
    info!(
        "game UI: uGUI read in {:.2} s; {} canvas scalers",
        start.elapsed().as_secs_f64(),
        scalers.len()
    );
    let camera = UiCamera::game(fov, NEAR, FAR);
    let mut sent: std::collections::HashSet<u64> = std::collections::HashSet::new();
    // Glyphs per font atlas when it was last sent.
    let mut sent_glyphs: HashMap<usize, usize> = HashMap::new();
    while let Ok(mut request) = requests.recv() {
        // Only the latest size matters.
        while let Ok(r) = requests.try_recv() {
            request = r;
        }
        let begin = Instant::now();
        let mut placements = HashMap::new();
        for (node, s) in &scalers {
            match place(s, &camera, fov, request.screen, DEFAULT_UI_SCALE) {
                Ok(p) => {
                    placements.insert(*node, p);
                }
                Err(e) => warn!("game UI: {}: {e}", tree.nodes[*node].path),
            }
        }
        let d = assets.ui_draw_placed(
            &layouts,
            &tree,
            &fonts,
            &set,
            &mut atlases,
            &tags.sorting_layers,
            &placements,
            &start_state,
        )?;
        let mut build = convert(&tree, &d, request.screen, fov, placements);
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
                "game UI: font atlas {} sent: {} glyphs, {}×{}",
                fonts.assets[font].name,
                a.glyphs.len(),
                a.width,
                a.height
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
        info!(
            "game UI: {} glyphs made so far in {:.3} s; atlas full {}",
            atlases.made, atlases.seconds, atlases.full
        );
        build.seconds = begin.elapsed().as_secs_f64();
        if builds.send(Ok(build)).is_err() {
            break;
        }
    }
    Ok(())
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
) -> UiBuild {
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
        let st = m
            .colors
            .get("_MainTex_ST")
            .copied()
            .unwrap_or([1.0, 1.0, 0.0, 0.0]);
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
            text,
        });
        // Draws that cover their whole canvas (a full-screen panel hides
        // everything behind it): logged with their colour.
        if let Some(p) = placements.get(&it.root) {
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
    UiBuild {
        screen,
        field_of_view,
        vertices,
        indices,
        draws,
        placements,
        textures: Vec::new(),
        seconds: 0.0,
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
    let _ = worker.requests.send(Request { screen: size });
}

fn receive_build(
    worker: Res<UiWorker>,
    mut frame: ResMut<GameUiFrame>,
    mut images: ResMut<Assets<Image>>,
) {
    let Ok(rx) = worker.builds.lock() else {
        return;
    };
    let mut latest = None;
    while let Ok(b) = rx.try_recv() {
        match b {
            Ok(b) => latest = Some(b),
            Err(e) => error!("game UI: {e}"),
        }
    }
    let Some(mut build) = latest else {
        return;
    };
    let mut map = (*frame.images).clone();
    for t in build.textures.drain(..) {
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
    info!(
        "game UI: built for {}×{} in {:.3} s; {} textures on the GPU",
        build.screen[0],
        build.screen[1],
        build.seconds,
        map.len()
    );
    frame.images = Arc::new(map);
    frame.build = Some(Arc::new(build));
    frame.generation += 1;
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

#[derive(Resource)]
struct UiPipelines {
    items_layout: BindGroupLayoutDescriptor,
    text_items_layout: BindGroupLayoutDescriptor,
    texture_layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
    text_shader: Handle<Shader>,
    /// By target format, blend and whether it draws text.
    by_key: HashMap<(TextureFormat, BlendKey, bool), CachedRenderPipelineId>,
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
        texture_layout,
        shader: asset_server.load("embedded://sn_client/game_ui.wgsl"),
        text_shader: asset_server.load("embedded://sn_client/game_ui_text.wgsl"),
        by_key: HashMap::new(),
    });
}

/// The pipelines each view needs: one per blend state and kind (image or
/// text) in the build.
#[derive(Component)]
struct UiPipelineIds(HashMap<(BlendKey, bool), CachedRenderPipelineId>);

fn pipeline_descriptor(
    p: &UiPipelines,
    format: TextureFormat,
    blend: BlendKey,
    text: bool,
) -> RenderPipelineDescriptor {
    let (items_layout, shader) = if text {
        (&p.text_items_layout, &p.text_shader)
    } else {
        (&p.items_layout, &p.shader)
    };
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
        layout: vec![items_layout.clone(), p.texture_layout.clone()],
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
    let mut blends: Vec<(BlendKey, bool)> = build
        .draws
        .iter()
        .map(|d| (d.blend, d.text.is_some()))
        .collect();
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
    if gpu.made_for == Some((screen, frame.generation)) {
        return;
    }
    gpu.made_for = Some((screen, frame.generation));
    let aspect = screen[0] as f32 / screen[1].max(1) as f32;
    let projection = clip_from_world(build.field_of_view, aspect);
    let mut items = Vec::with_capacity(build.draws.len());
    let mut text_items = Vec::new();
    let mut instances = Vec::with_capacity(build.draws.len());
    let pixel = Vec4::new(
        screen[0] as f32,
        screen[1] as f32,
        projection.x_axis.x,
        projection.y_axis.y,
    );
    for d in &build.draws {
        let Some(p) = build.placements.get(&d.root) else {
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
        instances.push(items.len() as u32);
        let alpha_only = frame.images.get(&d.texture).is_some_and(|(_, a)| *a);
        items.push(GpuItem {
            clip_from_canvas: clip_from_canvas(p, build.field_of_view, aspect),
            color: Vec4::from_array(d.color),
            st: Vec4::from_array(d.st),
            texture_add: if alpha_only {
                Vec4::new(1.0, 1.0, 1.0, 0.0)
            } else {
                Vec4::ZERO
            },
            clip_rect: Vec4::from_array(d.clip),
            softness: Vec4::new(d.softness[0], d.softness[1], d.flags as f32, 0.0),
            pixel,
        });
    }
    // Storage buffers may not be empty.
    if items.is_empty() {
        items.push(GpuItem::default());
    }
    if text_items.is_empty() {
        text_items.push(GpuTextItem::default());
    }
    gpu.items.set(GpuItems { items });
    gpu.items.write_buffer(&device, &queue);
    gpu.text_items.set(GpuTextItems { items: text_items });
    gpu.text_items.write_buffer(&device, &queue);
    gpu.instances = instances;
    let spent = begin.elapsed().as_secs_f64();
    gpu.cpu.0 += spent;
    gpu.cpu.1 += 1;
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
    let begin = Instant::now();
    let (target, ids) = view.into_inner();
    let Some(frame) = frame else {
        return;
    };
    let Some(build) = frame.build.clone() else {
        return;
    };
    let (Some(vertices), Some(indices), Some(items), Some(text_items)) = (
        &gpu.vertices,
        &gpu.indices,
        gpu.items.binding(),
        gpu.text_items.binding(),
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
    let texture_layout = pipeline_cache.get_bind_group_layout(&pipelines.texture_layout);
    let mut texture_groups = HashMap::new();
    for d in &build.draws {
        texture_groups.entry(d.texture).or_insert_with(|| {
            let handle = frame
                .images
                .get(&d.texture)
                .map_or(&frame.white, |(h, _)| h);
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
        let text = d.text.is_some();
        let (Some(id), Some(Some(group)), Some(&i)) = (
            ids.0.get(&(d.blend, text)),
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
        if current_kind != Some(text) {
            let items = if text {
                &text_items_group
            } else {
                &items_group
            };
            pass.set_bind_group(0, items, &[]);
            current_kind = Some(text);
        }
        pass.set_bind_group(1, group, &[]);
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
