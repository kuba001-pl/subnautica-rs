//! Effect meshes drawn as the game draws them (M7g3; `docs/DESIGN.md`
//! § 4.2, `docs/formats/materials.md`): after its fog, each fogging itself
//! at its own distance, into its two WBOIT targets (weighted blended
//! order-independent transparency), composited by the game's `WBOIT`
//! image effect after the sun shafts.
//!
//! Drawn this way: `UWE/Particles/WBOIT-FakeVolumetricLight` (the glows on
//! ion crystal pedestals and under Precursor lights, `effects.wgsl`) and the
//! `UWE/Particles/UBER` materials in the variants we decoded (the Precursor
//! consoles' holograms, `effects_uber.wgsl`; the client checks each
//! material's keywords and render state). The worker marks their
//! parts; the main world spawns them as [`EffectPart`] entities; here they
//! are extracted every frame, their meshes uploaded once into our own
//! buffers, and drawn in two passes:
//! 1. accumulation into targets A and B cleared to (0, 0, 0, 1), with the
//!    game's blend (colour `One One`, alpha `Zero OneMinusSrcAlpha`, one
//!    blend for all targets), no depth write, both sides; the scene's depth
//!    is tested in the shader and gives the soft edges (`effects.wgsl`);
//! 2. the composite, `Hidden/WBOIT Composite` without keywords
//!    (`effects_composite.wgsl`), skipped when nothing was drawn.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only, texture_2d, texture_depth_2d, texture_depth_2d_multisampled,
    uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BlendComponent,
    BlendFactor, BlendOperation, BlendState, Buffer, BufferInitDescriptor, BufferUsages,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, Extent3d, FragmentState, IndexFormat,
    LoadOp, Operations, PipelineCache, PrimitiveState, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderStages, ShaderType, StorageBuffer, StoreOp, TextureDescriptor, TextureDimension,
    TextureFormat, TextureSampleType, TextureUsages, UniformBuffer, VertexAttribute, VertexFormat,
    VertexState, VertexStepMode,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::{CachedTexture, GpuImage, TextureCache};
use bevy::render::view::{
    ExtractedView, Msaa, ViewDepthTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms,
};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};

use crate::objects::MeshData;
use crate::sky::SkyState;
use crate::sun_shafts::SunShaftsPass;
use crate::textures::linear;
use crate::water::{WaterFog, WaterWorld};

/// The game's WBOIT targets: `ARGBHalf`, linear.
const WBOIT_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
/// The `WBOIT` component on the main camera (main scene, M7g3):
/// `useDepthWeighting` 1, `depthWeightingSharpness` 0.1.
const DEPTH_WEIGHTING: f32 = 1.0;
const DEPTH_WEIGHTING_SHARPNESS: f32 = 0.1;
/// Floats per vertex in our effect vertex buffer: position, normal,
/// colour, uv.
const VERTEX_FLOATS: usize = 12;

/// An effect material's values (`UWE/Particles/WBOIT-FakeVolumetricLight`),
/// the material's own or the shader's defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EffectValues {
    /// `_Color` as stored (sRGB).
    pub color: [f32; 4],
    pub intensity: f32,
    pub fresnel_fade: f32,
    pub fresnel_pow: f32,
    pub clip_offset: f32,
    pub clip_fade: f32,
    pub offset: f32,
    pub fallof: f32,
    pub inv_fade: f32,
}

/// A `UWE/Particles/UBER` material's values in the variants we draw
/// (`docs/formats/materials.md` § `UWE/Particles/UBER` meshes): keywords
/// `FX_ADDFOG FX_SCROLL WBOIT`, with or without `FX_MULMAP` and
/// `FX_FRESNELCLIP` (the consoles' holograms), or with `FX_DEFORM FX_MULMAP
/// FX_SOFTEDGES` and with or without `FX_REFRACTMAP` (the doors' force
/// fields).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ParticlesValues {
    /// `_Color` as stored (sRGB; a Color property, so Unity makes it
    /// linear).
    pub color: [f32; 4],
    /// `_ColorStrength`, `_ColorStrengthAtNight`: Vector properties, used as
    /// stored (no colour conversion).
    pub strength: [f32; 4],
    pub strength_night: [f32; 4],
    /// `_MainTex_ST`, `_MainTex2_ST` (scale x, y, offset x, y).
    pub main_st: [f32; 4],
    pub main2_st: [f32; 4],
    /// `_MainTex_Speed.xy`, `_MainTex2_Speed.xy`.
    pub speed: [f32; 4],
    pub fresnel_fade: f32,
    pub fresnel_pow: f32,
    pub cutoff: f32,
    /// `FX_FRESNELCLIP`, `FX_MULMAP`, `FX_SOFTEDGES`, `FX_DEFORM`,
    /// `FX_REFRACTMAP`.
    pub fresnel_clip: bool,
    pub mul_map: bool,
    pub soft_edges: bool,
    pub deform: bool,
    pub refract: bool,
    /// `_InvFade`, `_DeformStrength`, `_RefractStrength` (the float; the
    /// materials also hold a vector of that name, which the programs don't
    /// read).
    pub inv_fade: f32,
    pub deform_strength: f32,
    pub refract_strength: f32,
    /// `_DeformMap_ST`, `_RefractMap_ST`.
    pub deform_st: [f32; 4],
    pub refract_st: [f32; 4],
    /// `_DeformMap_Speed.xy`, `_RefractMap_Speed.xy`.
    pub speed2: [f32; 4],
    /// Texture ids of `_MainTex`, `_MainTex2`, `_DeformMap`, `_RefractMap`
    /// (`None`: the shader's default, white).
    pub textures: [Option<u32>; 4],
}

/// Which of our effect shaders draws a material, with its values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EffectLook {
    /// `UWE/Particles/WBOIT-FakeVolumetricLight` (`effects.wgsl`).
    Glow(EffectValues),
    /// `UWE/Particles/UBER` (`effects_uber.wgsl`).
    Particles(ParticlesValues),
}

/// A placed effect sub-mesh; its transform is the world placement.
#[derive(Component, Clone)]
#[require(Transform, Visibility)]
pub(crate) struct EffectPart {
    /// Id in [`EffectMeshes`].
    pub mesh: u32,
    pub look: EffectLook,
    /// `_MainTex`, `_MainTex2`, `_DeformMap`, `_RefractMap` for
    /// [`EffectLook::Particles`] (white where the material has none); unused
    /// by the glows.
    pub textures: [Handle<Image>; PARTICLES_TEXTURES],
}

/// Textures of a `UWE/Particles/UBER` draw.
pub(crate) const PARTICLES_TEXTURES: usize = 4;

/// The effect meshes received so far (Bevy's coordinates), by id.
#[derive(Resource, Default, Clone, ExtractResource)]
pub(crate) struct EffectMeshes(pub HashMap<u32, Arc<MeshData>>);

/// One drawn effect; must match `Instance` in `effects.wgsl`.
#[derive(Clone, Copy, Default, ShaderType)]
struct EffectInstance {
    world_from_local: Mat4,
    local_from_world: Mat4,
    /// `_Color`, linear.
    color: Vec4,
    /// `_FresnelFade`, `_FresnelPow`, `_ClipOffset`, `_ClipFade`.
    fresnel: Vec4,
    /// `_Intensity`, `_Offset`, `_Fallof`, `_InvFade`.
    falloff: Vec4,
}

#[derive(Clone, Default, ShaderType)]
struct EffectInstances {
    #[shader(size(runtime))]
    items: Vec<EffectInstance>,
}

/// One drawn `UWE/Particles/UBER` mesh; must match `Instance` in
/// `effects_uber.wgsl`.
#[derive(Clone, Copy, Default, ShaderType)]
struct ParticlesInstance {
    world_from_local: Mat4,
    local_from_world: Mat4,
    /// `_Color`, linear.
    color: Vec4,
    /// `_ColorStrength`, `_ColorStrengthAtNight`, as stored.
    strength: Vec4,
    strength_night: Vec4,
    main_st: Vec4,
    main2_st: Vec4,
    deform_st: Vec4,
    refract_st: Vec4,
    /// `_MainTex_Speed.xy`, `_MainTex2_Speed.xy`.
    speed: Vec4,
    /// `_DeformMap_Speed.xy`, `_RefractMap_Speed.xy`.
    speed2: Vec4,
    /// `_FresnelFade`, `_FresnelPow`, `_Cutoff`, flags (1 `FX_FRESNELCLIP`,
    /// 2 `FX_MULMAP`, 4 `FX_SOFTEDGES`, 8 `FX_DEFORM`, 16 `FX_REFRACTMAP`).
    params: Vec4,
    /// `_InvFade`, `_DeformStrength`, `_RefractStrength`, unused.
    strengths: Vec4,
}

#[derive(Clone, Default, ShaderType)]
struct ParticlesInstances {
    #[shader(size(runtime))]
    items: Vec<ParticlesInstance>,
}

/// Must match `Globals` in `effects.wgsl` and `effects_uber.wgsl`.
#[derive(Clone, Copy, Default, ShaderType)]
struct EffectGlobals {
    /// Depth weighting on (1) or off, its sharpness, our light unit, unused.
    wboit: Vec4,
    /// `_Time.y` (s), `_UweLocalLightScalar`, unused, unused.
    misc: Vec4,
}

/// This frame's effects, in the render world.
#[derive(Resource, Default)]
struct ExtractedEffects {
    instances: Vec<EffectInstance>,
    meshes: Vec<u32>,
    particles: Vec<ParticlesInstance>,
    /// Per particles instance: its mesh and its textures.
    particle_draws: Vec<(u32, [AssetId<Image>; PARTICLES_TEXTURES])>,
    light_unit: f32,
    time: f32,
    local_light: f32,
}

impl ExtractedEffects {
    fn is_empty(&self) -> bool {
        self.instances.is_empty() && self.particles.is_empty()
    }
}

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "effects.wgsl");
        bevy::asset::embedded_asset!(app, "effects_uber.wgsl");
        bevy::asset::embedded_asset!(app, "effects_composite.wgsl");
        app.init_resource::<EffectMeshes>()
            .add_plugins(ExtractResourcePlugin::<EffectMeshes>::default())
            .add_systems(Update, log_effect_count);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<ExtractedEffects>()
            .init_resource::<GpuEffectMeshes>()
            .add_systems(RenderStartup, init_gpu)
            .add_systems(ExtractSchedule, extract_effects)
            .add_systems(
                Render,
                (
                    prepare_pipelines.in_set(RenderSystems::Prepare),
                    prepare_meshes.in_set(RenderSystems::PrepareResources),
                    prepare_buffers.in_set(RenderSystems::PrepareResources),
                    prepare_targets.in_set(RenderSystems::PrepareResources),
                ),
            )
            .add_systems(
                Core3d,
                effects_pass
                    .in_set(Core3dSystems::PostProcess)
                    .after(SunShaftsPass)
                    .before(tonemapping),
            );
    }
}

/// Logs how many effect parts exist whenever the number changes (they
/// arrive with their objects, so rarely).
fn log_effect_count(
    parts: Query<(), With<EffectPart>>,
    meshes: Res<EffectMeshes>,
    mut last: Local<usize>,
) {
    let count = parts.iter().count();
    if count != *last {
        *last = count;
        info!("effects: {count} parts, {} meshes", meshes.0.len());
    }
}

fn extract_effects(
    mut extracted: ResMut<ExtractedEffects>,
    parts: Extract<Query<(&EffectPart, &GlobalTransform, &InheritedVisibility)>>,
    water: Extract<Option<Res<WaterWorld>>>,
    sky: Extract<Option<Res<SkyState>>>,
    time: Extract<Res<Time>>,
) {
    extracted.instances.clear();
    extracted.meshes.clear();
    extracted.particles.clear();
    extracted.particle_draws.clear();
    for (part, transform, visible) in &parts {
        if !visible.get() {
            continue;
        }
        let world_from_local = transform.to_matrix();
        let local_from_world = world_from_local.inverse();
        match part.look {
            EffectLook::Glow(v) => {
                extracted.instances.push(EffectInstance {
                    world_from_local,
                    local_from_world,
                    color: linear(v.color),
                    fresnel: Vec4::new(v.fresnel_fade, v.fresnel_pow, v.clip_offset, v.clip_fade),
                    falloff: Vec4::new(v.intensity, v.offset, v.fallof, v.inv_fade),
                });
                extracted.meshes.push(part.mesh);
            }
            EffectLook::Particles(v) => {
                let flags = u32::from(v.fresnel_clip)
                    | (u32::from(v.mul_map) << 1)
                    | (u32::from(v.soft_edges) << 2)
                    | (u32::from(v.deform) << 3)
                    | (u32::from(v.refract) << 4);
                extracted.particles.push(ParticlesInstance {
                    world_from_local,
                    local_from_world,
                    color: linear(v.color),
                    strength: Vec4::from(v.strength),
                    strength_night: Vec4::from(v.strength_night),
                    main_st: Vec4::from(v.main_st),
                    main2_st: Vec4::from(v.main2_st),
                    deform_st: Vec4::from(v.deform_st),
                    refract_st: Vec4::from(v.refract_st),
                    speed: Vec4::from(v.speed),
                    speed2: Vec4::from(v.speed2),
                    params: Vec4::new(v.fresnel_fade, v.fresnel_pow, v.cutoff, flags as f32),
                    strengths: Vec4::new(v.inv_fade, v.deform_strength, v.refract_strength, 0.0),
                });
                let ids = part.textures.each_ref().map(Handle::id);
                extracted.particle_draws.push((part.mesh, ids));
            }
        }
    }
    extracted.light_unit = water.as_ref().map_or(1.0, |w| w.light_unit);
    // `_Time.y`: seconds since the level loaded (any start gives the game's
    // look: only its fraction scrolls the textures).
    extracted.time = time.elapsed_secs();
    // By day when the sky is unknown.
    extracted.local_light = sky.as_ref().map_or(1.0, |s| s.local_light);
}

struct GpuEffectMesh {
    vertices: Buffer,
    indices: Buffer,
    index_count: u32,
}

#[derive(Resource, Default)]
struct GpuEffectMeshes(HashMap<u32, GpuEffectMesh>);

/// Interleaved vertices (position, normal, colour, uv). Missing normals
/// point up, missing colours are white (**hypothesis**: what Unity binds
/// for a missing colour channel), missing uvs are 0.
fn vertex_bytes(data: &MeshData) -> Vec<u8> {
    let n = data.positions.len();
    let mut floats = Vec::with_capacity(n * VERTEX_FLOATS);
    for i in 0..n {
        floats.extend_from_slice(&data.positions[i]);
        floats.extend_from_slice(data.normals.get(i).unwrap_or(&[0.0, 1.0, 0.0]));
        floats.extend_from_slice(data.colors.get(i).unwrap_or(&[1.0; 4]));
        floats.extend_from_slice(data.uvs.get(i).unwrap_or(&[0.0; 2]));
    }
    floats.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn prepare_meshes(
    meshes: Option<Res<EffectMeshes>>,
    mut gpu: ResMut<GpuEffectMeshes>,
    device: Res<RenderDevice>,
) {
    let Some(meshes) = meshes else {
        return;
    };
    for (id, data) in &meshes.0 {
        if gpu.0.contains_key(id) || data.indices.is_empty() {
            continue;
        }
        let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("effect_vertices"),
            contents: &vertex_bytes(data),
            usage: BufferUsages::VERTEX,
        });
        let index_bytes: Vec<u8> = data.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        let indices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("effect_indices"),
            contents: &index_bytes,
            usage: BufferUsages::INDEX,
        });
        gpu.0.insert(
            *id,
            GpuEffectMesh {
                vertices,
                indices,
                index_count: data.indices.len() as u32,
            },
        );
    }
}

/// The frame's instance and global buffers.
#[derive(Resource)]
struct EffectBuffers {
    instances: StorageBuffer<EffectInstances>,
    particles: StorageBuffer<ParticlesInstances>,
    globals: UniformBuffer<EffectGlobals>,
    /// Per glow instance: its mesh id.
    meshes: Vec<u32>,
    /// Per particles instance: its mesh id and textures.
    particle_draws: Vec<(u32, [AssetId<Image>; PARTICLES_TEXTURES])>,
}

fn prepare_buffers(
    mut commands: Commands,
    extracted: Res<ExtractedEffects>,
    buffers: Option<ResMut<EffectBuffers>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let mut buffers = match buffers {
        Some(b) => b,
        None => {
            commands.insert_resource(EffectBuffers {
                instances: StorageBuffer::default(),
                particles: StorageBuffer::default(),
                globals: UniformBuffer::default(),
                meshes: Vec::new(),
                particle_draws: Vec::new(),
            });
            return;
        }
    };
    buffers.meshes.clone_from(&extracted.meshes);
    buffers.particle_draws.clone_from(&extracted.particle_draws);
    if !extracted.instances.is_empty() {
        buffers.instances.set(EffectInstances {
            items: extracted.instances.clone(),
        });
        buffers.instances.write_buffer(&device, &queue);
    }
    if !extracted.particles.is_empty() {
        buffers.particles.set(ParticlesInstances {
            items: extracted.particles.clone(),
        });
        buffers.particles.write_buffer(&device, &queue);
    }
    if extracted.is_empty() {
        return;
    }
    buffers.globals.set(EffectGlobals {
        wboit: Vec4::new(
            DEPTH_WEIGHTING,
            DEPTH_WEIGHTING_SHARPNESS,
            extracted.light_unit,
            0.0,
        ),
        misc: Vec4::new(extracted.time, extracted.local_light, 0.0, 0.0),
    });
    buffers.globals.write_buffer(&device, &queue);
}

/// A view's WBOIT targets A and B.
#[derive(Component)]
struct EffectTargets {
    a: CachedTexture,
    b: CachedTexture,
}

fn prepare_targets(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    extracted: Res<ExtractedEffects>,
    views: Query<(Entity, &ExtractedView), With<WaterFog>>,
) {
    if extracted.is_empty() {
        return;
    }
    for (entity, view) in &views {
        let target = |label: &'static str, cache: &mut TextureCache| {
            cache.get(
                &device,
                TextureDescriptor {
                    label: Some(label),
                    size: Extent3d {
                        width: view.viewport.z.max(1),
                        height: view.viewport.w.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: WBOIT_FORMAT,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        let a = target("wboit_a", &mut cache);
        let b = target("wboit_b", &mut cache);
        commands.entity(entity).insert(EffectTargets { a, b });
    }
}

#[derive(Resource)]
struct EffectPipelines {
    /// Accumulation, single-sampled and multisampled scene depth.
    accumulate_layouts: [BindGroupLayoutDescriptor; 2],
    /// The same for `UWE/Particles/UBER`, and its textures (group 1).
    particles_layouts: [BindGroupLayoutDescriptor; 2],
    particles_textures_layout: BindGroupLayoutDescriptor,
    composite_layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    shader: Handle<Shader>,
    particles_shader: Handle<Shader>,
    composite_shader: Handle<Shader>,
    fullscreen: FullscreenShader,
    accumulate: HashMap<bool, CachedRenderPipelineId>,
    particles: HashMap<bool, CachedRenderPipelineId>,
    composite: HashMap<TextureFormat, CachedRenderPipelineId>,
}

fn init_gpu(
    mut commands: Commands,
    device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
) {
    let depth = |multisampled: bool| {
        if multisampled {
            texture_depth_2d_multisampled()
        } else {
            texture_depth_2d()
        }
    };
    let accumulate_layout = |multisampled: bool| {
        BindGroupLayoutDescriptor::new(
            "effects_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    uniform_buffer::<ViewUniform>(true),
                    uniform_buffer::<WaterFog>(true),
                    uniform_buffer::<EffectGlobals>(false),
                    storage_buffer_read_only::<EffectInstances>(false),
                    depth(multisampled),
                ),
            ),
        )
    };
    let particles_layout = |multisampled: bool| {
        BindGroupLayoutDescriptor::new(
            "effects_particles_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    uniform_buffer::<ViewUniform>(true),
                    uniform_buffer::<WaterFog>(true),
                    uniform_buffer::<EffectGlobals>(false),
                    storage_buffer_read_only::<ParticlesInstances>(false),
                    depth(multisampled),
                ),
            ),
        )
    };
    let float = || texture_2d(TextureSampleType::Float { filterable: true });
    // Each texture with its own sampler, as the game's textures have their
    // own wrap and filter settings.
    let particles_textures_layout = BindGroupLayoutDescriptor::new(
        "effects_particles_textures_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                float(),
                sampler(SamplerBindingType::Filtering),
                float(),
                sampler(SamplerBindingType::Filtering),
                float(),
                sampler(SamplerBindingType::Filtering),
                float(),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let composite_layout = BindGroupLayoutDescriptor::new(
        "effects_composite_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                float(),
                float(),
                float(),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    commands.insert_resource(EffectPipelines {
        accumulate_layouts: [accumulate_layout(false), accumulate_layout(true)],
        particles_layouts: [particles_layout(false), particles_layout(true)],
        particles_textures_layout,
        composite_layout,
        // Unity's render textures sample bilinearly, clamped.
        sampler: device.create_sampler(&SamplerDescriptor {
            label: Some("effects_composite"),
            mag_filter: bevy::render::render_resource::FilterMode::Linear,
            min_filter: bevy::render::render_resource::FilterMode::Linear,
            ..default()
        }),
        shader: asset_server.load("embedded://sn_client/effects.wgsl"),
        particles_shader: asset_server.load("embedded://sn_client/effects_uber.wgsl"),
        composite_shader: asset_server.load("embedded://sn_client/effects_composite.wgsl"),
        fullscreen: fullscreen.clone(),
        accumulate: HashMap::new(),
        particles: HashMap::new(),
        composite: HashMap::new(),
    });
}

#[derive(Component)]
struct EffectPipelineIds {
    accumulate: CachedRenderPipelineId,
    particles: CachedRenderPipelineId,
    composite: CachedRenderPipelineId,
    multisampled: bool,
}

/// An accumulation pipeline into the WBOIT targets: our effect vertex
/// buffer, the game's blend for these shaders, both sides, no depth buffer.
fn accumulate_descriptor(
    label: &'static str,
    layout: Vec<BindGroupLayoutDescriptor>,
    shader: &Handle<Shader>,
    multisampled: bool,
) -> RenderPipelineDescriptor {
    let shader_defs = if multisampled {
        vec!["MULTISAMPLED".into()]
    } else {
        Vec::new()
    };
    let attribute = |format, offset, shader_location| VertexAttribute {
        format,
        offset,
        shader_location,
    };
    // The game's blend, the same for every target (no separate blend):
    // colour One One, alpha Zero OneMinusSrcAlpha. For the glows the
    // shader's own; for `UWE/Particles/UBER` the materials' `_SrcBlend` 1,
    // `_DstBlend` 1, `_SrcBlend2` 0, `_DstBlend2` 10 (the client draws no
    // other values here).
    let blend = BlendState {
        color: BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent {
            src_factor: BlendFactor::Zero,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        },
    };
    let target = Some(ColorTargetState {
        format: WBOIT_FORMAT,
        blend: Some(blend),
        write_mask: ColorWrites::ALL,
    });
    RenderPipelineDescriptor {
        label: Some(label.into()),
        layout,
        vertex: VertexState {
            shader: shader.clone(),
            shader_defs: shader_defs.clone(),
            entry_point: Some("vertex".into()),
            buffers: vec![VertexBufferLayout {
                array_stride: (VERTEX_FLOATS * 4) as u64,
                step_mode: VertexStepMode::Vertex,
                attributes: vec![
                    attribute(VertexFormat::Float32x3, 0, 0),
                    attribute(VertexFormat::Float32x3, 12, 1),
                    attribute(VertexFormat::Float32x4, 24, 2),
                    attribute(VertexFormat::Float32x2, 40, 3),
                ],
            }],
        },
        // Both sides (Cull Off); no depth buffer: the scene's depth is
        // tested in the shader (no ZWrite).
        primitive: PrimitiveState {
            cull_mode: None,
            ..default()
        },
        depth_stencil: None,
        fragment: Some(FragmentState {
            shader: shader.clone(),
            shader_defs,
            entry_point: Some("fragment".into()),
            targets: vec![target.clone(), target],
        }),
        ..default()
    }
}

fn prepare_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut pipelines: ResMut<EffectPipelines>,
    views: Query<(Entity, &ExtractedView, &Msaa), With<WaterFog>>,
) {
    for (entity, view, msaa) in &views {
        let multisampled = msaa.samples() > 1;
        let ms = usize::from(multisampled);
        let accumulate = match pipelines.accumulate.get(&multisampled) {
            Some(id) => *id,
            None => {
                let descriptor = accumulate_descriptor(
                    "effects_accumulate_pipeline",
                    vec![pipelines.accumulate_layouts[ms].clone()],
                    &pipelines.shader,
                    multisampled,
                );
                let id = pipeline_cache.queue_render_pipeline(descriptor);
                pipelines.accumulate.insert(multisampled, id);
                id
            }
        };
        let particles = match pipelines.particles.get(&multisampled) {
            Some(id) => *id,
            None => {
                let descriptor = accumulate_descriptor(
                    "effects_particles_pipeline",
                    vec![
                        pipelines.particles_layouts[ms].clone(),
                        pipelines.particles_textures_layout.clone(),
                    ],
                    &pipelines.particles_shader,
                    multisampled,
                );
                let id = pipeline_cache.queue_render_pipeline(descriptor);
                pipelines.particles.insert(multisampled, id);
                id
            }
        };
        let composite = match pipelines.composite.get(&view.target_format) {
            Some(id) => *id,
            None => {
                let descriptor = RenderPipelineDescriptor {
                    label: Some("effects_composite_pipeline".into()),
                    layout: vec![pipelines.composite_layout.clone()],
                    vertex: pipelines.fullscreen.to_vertex_state(),
                    fragment: Some(FragmentState {
                        shader: pipelines.composite_shader.clone(),
                        targets: vec![Some(ColorTargetState {
                            format: view.target_format,
                            blend: None,
                            write_mask: ColorWrites::ALL,
                        })],
                        ..default()
                    }),
                    ..default()
                };
                let id = pipeline_cache.queue_render_pipeline(descriptor);
                pipelines.composite.insert(view.target_format, id);
                id
            }
        };
        commands.entity(entity).insert(EffectPipelineIds {
            accumulate,
            particles,
            composite,
            multisampled,
        });
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // a render system
fn effects_pass(
    view: ViewQuery<(
        &ViewTarget,
        &ViewDepthTexture,
        &ViewUniformOffset,
        &DynamicUniformIndex<WaterFog>,
        &EffectPipelineIds,
        Option<&EffectTargets>,
    )>,
    pipeline_cache: Res<PipelineCache>,
    pipelines: Res<EffectPipelines>,
    buffers: Option<Res<EffectBuffers>>,
    meshes: Res<GpuEffectMeshes>,
    images: Res<RenderAssets<GpuImage>>,
    view_uniforms: Res<ViewUniforms>,
    fog_uniforms: Res<ComponentUniforms<WaterFog>>,
    device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (target, depth, view_offset, fog_index, ids, targets) = view.into_inner();
    let (Some(buffers), Some(targets)) = (buffers, targets) else {
        return;
    };
    if buffers.meshes.is_empty() && buffers.particle_draws.is_empty() {
        return;
    }
    let (Some(accumulate), Some(particles), Some(composite)) = (
        pipeline_cache.get_render_pipeline(ids.accumulate),
        pipeline_cache.get_render_pipeline(ids.particles),
        pipeline_cache.get_render_pipeline(ids.composite),
    ) else {
        return;
    };
    let (Some(view_binding), Some(fog_binding), Some(globals)) = (
        view_uniforms.uniforms.binding(),
        fog_uniforms.uniforms().binding(),
        buffers.globals.binding(),
    ) else {
        return;
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(ctx.command_encoder(), "effects");

    // 1. Accumulation into A and B, cleared as the game does.
    let ms = usize::from(ids.multisampled);
    let glow_group = buffers
        .instances
        .binding()
        .filter(|_| !buffers.meshes.is_empty())
        .map(|instances| {
            device.create_bind_group(
                "effects_bind_group",
                &pipeline_cache.get_bind_group_layout(&pipelines.accumulate_layouts[ms]),
                &BindGroupEntries::sequential((
                    view_binding.clone(),
                    fog_binding.clone(),
                    globals.clone(),
                    instances,
                    depth.view(),
                )),
            )
        });
    let particles_group = buffers
        .particles
        .binding()
        .filter(|_| !buffers.particle_draws.is_empty())
        .map(|instances| {
            device.create_bind_group(
                "effects_particles_bind_group",
                &pipeline_cache.get_bind_group_layout(&pipelines.particles_layouts[ms]),
                &BindGroupEntries::sequential((
                    view_binding.clone(),
                    fog_binding.clone(),
                    globals.clone(),
                    instances,
                    depth.view(),
                )),
            )
        });
    // One texture group per distinct texture set (the consoles share
    // theirs); sets whose images are not all on the GPU yet are skipped.
    let textures_layout =
        pipeline_cache.get_bind_group_layout(&pipelines.particles_textures_layout);
    let mut texture_groups: HashMap<[AssetId<Image>; PARTICLES_TEXTURES], Option<BindGroup>> =
        HashMap::new();
    for (_, set) in &buffers.particle_draws {
        texture_groups.entry(*set).or_insert_with(|| {
            let [Some(a), Some(b), Some(c), Some(d)] = set.map(|id| images.get(id)) else {
                return None;
            };
            Some(device.create_bind_group(
                "effects_particles_textures",
                &textures_layout,
                &BindGroupEntries::sequential((
                    &a.texture_view,
                    &a.sampler,
                    &b.texture_view,
                    &b.sampler,
                    &c.texture_view,
                    &c.sampler,
                    &d.texture_view,
                    &d.sampler,
                )),
            ))
        });
    }
    let clear = LoadOp::Clear(LinearRgba::new(0.0, 0.0, 0.0, 1.0).into());
    let attachment = |view| {
        Some(RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: clear,
                store: StoreOp::Store,
            },
        })
    };
    {
        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("effects_accumulate"),
                color_attachments: &[
                    attachment(&targets.a.default_view),
                    attachment(&targets.b.default_view),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        let offsets = [view_offset.offset, fog_index.index()];
        if let Some(group) = &glow_group {
            pass.set_pipeline(accumulate);
            pass.set_bind_group(0, group, &offsets);
            for (i, id) in buffers.meshes.iter().enumerate() {
                let Some(mesh) = meshes.0.get(id) else {
                    continue;
                };
                let i = i as u32;
                pass.set_vertex_buffer(0, (*mesh.vertices).slice(..));
                pass.set_index_buffer((*mesh.indices).slice(..), IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, i..i + 1);
            }
        }
        if let Some(group) = &particles_group {
            pass.set_pipeline(particles);
            pass.set_bind_group(0, group, &offsets);
            for (i, (id, set)) in buffers.particle_draws.iter().enumerate() {
                let (Some(mesh), Some(Some(textures))) =
                    (meshes.0.get(id), texture_groups.get(set))
                else {
                    continue;
                };
                let i = i as u32;
                pass.set_bind_group(1, textures, &[]);
                pass.set_vertex_buffer(0, (*mesh.vertices).slice(..));
                pass.set_index_buffer((*mesh.indices).slice(..), IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, i..i + 1);
            }
        }
    }

    // 2. The composite over the image so far.
    let post = target.post_process_write();
    let group = device.create_bind_group(
        "effects_composite_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipelines.composite_layout),
        &BindGroupEntries::sequential((
            post.source,
            &targets.a.default_view,
            &targets.b.default_view,
            &pipelines.sampler,
        )),
    );
    {
        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("effects_composite"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: post.destination,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations::default(),
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        pass.set_pipeline(composite);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    span.end(ctx.command_encoder());
}

#[cfg(test)]
mod tests {
    use super::*;

    // The decoded formulas (`docs/formats/materials.md` § Fake volumetric
    // lights), as `effects.wgsl` and `effects_composite.wgsl` compute them;
    // the tests check the formulas, not the WGSL.

    /// The glow's α before fog: falloff (vertex alpha `a`), soft edges
    /// (`gap`: scene depth − glow depth), Fresnel (`facing` = |v · n|), near
    /// fade (`eye`, `near`).
    fn glow_alpha(v: &EffectValues, a: f32, gap: f32, facing: f32, eye: f32, near: f32) -> f32 {
        let t = ((1.0 - a - v.offset + 0.01) / (v.fallof + 0.01)).clamp(0.0, 1.0);
        let fall = t * t * (3.0 - 2.0 * t) * a;
        let soft = (gap * v.inv_fade).clamp(0.0, 1.0);
        let fres = facing.powf(v.fresnel_pow) * v.fresnel_fade;
        let clip = ((eye - v.clip_offset - near) / (v.clip_fade + v.clip_offset)).min(1.0);
        (fall * soft * fres * clip).clamp(0.0, 1.0) * v.color[3] * v.intensity
    }

    /// The fade over the glow's path through fogged water.
    fn fog_fade(water_length: f32) -> f32 {
        (10.0 - 0.08 * water_length).clamp(0.0, 1.0)
    }

    /// Accumulates glows (colour, α, eye depth) into A and B as the
    /// game's blend does, then composites over `scene`.
    fn wboit(glows: &[(f32, f32, f32)], scene: f32) -> f32 {
        let (mut a_rgb, mut a_a, mut b) = (0.0, 1.0, 0.0);
        for &(c, alpha, eye) in glows {
            let weight = (-eye * DEPTH_WEIGHTING_SHARPNESS).exp().clamp(0.01, 1.0);
            let w = DEPTH_WEIGHTING * (alpha * weight - 1.0) + 1.0;
            a_rgb += w * alpha * c;
            a_a *= 1.0 - alpha;
            b += alpha * w;
        }
        let keep = 1.0 - a_a;
        keep * a_rgb / b.clamp(1e-4, 5e4) + a_a * scene
    }

    /// `x_AtmoLight_Sphere`'s values (logged by the client, M7g3).
    const SPHERE: EffectValues = EffectValues {
        color: [1.0; 4],
        intensity: 1.0,
        fresnel_fade: 0.75,
        fresnel_pow: 4.0,
        clip_offset: 0.0,
        clip_fade: 1.0,
        offset: 0.0,
        fallof: 0.0,
        inv_fade: 2.0,
    };

    #[test]
    fn glow_alpha_follows_the_decoded_terms() {
        // Facing the camera, far from the scene and the camera: the Fresnel
        // fade alone (0.75).
        assert!((glow_alpha(&SPHERE, 1.0, 10.0, 1.0, 10.0, 0.1) - 0.75).abs() < 1e-6);
        // Gone at the silhouette, at the scene, at the near plane, and where
        // the vertex alpha is 0.
        assert_eq!(glow_alpha(&SPHERE, 1.0, 10.0, 0.0, 10.0, 0.1), 0.0);
        assert_eq!(glow_alpha(&SPHERE, 1.0, 0.0, 1.0, 10.0, 0.1), 0.0);
        assert_eq!(glow_alpha(&SPHERE, 1.0, 10.0, 1.0, 0.1, 0.1), 0.0);
        assert_eq!(glow_alpha(&SPHERE, 0.0, 10.0, 1.0, 10.0, 0.1), 0.0);
        // Soft edges reach full strength 0.5 m from the scene (_InvFade 2).
        let half = glow_alpha(&SPHERE, 1.0, 0.25, 1.0, 10.0, 0.1);
        assert!((half - 0.375).abs() < 1e-6);
        // Not clamped after _Intensity, as in the game.
        let bright = EffectValues {
            intensity: 2.0,
            fresnel_fade: 1.0,
            ..SPHERE
        };
        assert_eq!(glow_alpha(&bright, 1.0, 10.0, 1.0, 10.0, 0.1), 2.0);
    }

    #[test]
    fn fog_fade_ends_between_112_5_and_125_m() {
        assert_eq!(fog_fade(0.0), 1.0);
        assert_eq!(fog_fade(112.5), 1.0);
        assert!((fog_fade(118.75) - 0.5).abs() < 1e-5);
        assert_eq!(fog_fade(125.0), 0.0);
    }

    #[test]
    fn one_glow_composites_as_alpha_blending() {
        for &(c, alpha, eye, scene) in &[(1.0, 0.5, 10.0, 0.2), (0.3, 0.9, 80.0, 1.0)] {
            let expected = alpha * c + (1.0 - alpha) * scene;
            assert!((wboit(&[(c, alpha, eye)], scene) - expected).abs() < 1e-5);
        }
        // Nothing drawn: the scene unchanged.
        assert_eq!(wboit(&[], 0.7), 0.7);
    }

    #[test]
    fn overlapping_glows_average_their_colours_by_weight() {
        // Two glows of equal α and depth: their mean colour, the scene
        // dimmed by both (1 − α).
        let out = wboit(&[(1.0, 0.5, 5.0), (0.0, 0.5, 5.0)], 0.0);
        assert!((out - 0.75 * 0.5).abs() < 1e-5);
        // Nearer glows weigh more (depth weighting on).
        let near_white = wboit(&[(1.0, 0.5, 1.0), (0.0, 0.5, 30.0)], 0.0);
        assert!(near_white > 0.375);
    }

    /// `UWE/Particles/UBER`'s colour and α before fog (`None`: discarded),
    /// as decoded (`effects_uber.wgsl`): `facing` = v · n, `vertex` the
    /// vertex colour, `tex`/`tex2` the samples, `night` = 1 − local light.
    fn particles(
        v: &ParticlesValues,
        facing: f32,
        vertex: [f32; 4],
        tex: [f32; 4],
        tex2: [f32; 4],
        night: f32,
    ) -> Option<([f32; 3], f32)> {
        let mut c = [
            2.0 * v.color[0],
            2.0 * v.color[1],
            2.0 * v.color[2],
            2.0 * v.color[3],
        ];
        if v.fresnel_clip {
            let rim = if v.fresnel_fade < 0.0 { 1.0 } else { 0.0 };
            let f = (rim - facing.abs()).abs().powf(v.fresnel_pow) * v.fresnel_fade.abs();
            c[3] *= f.min(1.0);
        }
        for i in 0..4 {
            c[i] *= vertex[i] * tex[i];
            if v.mul_map {
                c[i] *= tex2[i];
            }
            c[i] *= v.strength[i] + (v.strength_night[i] - v.strength[i]) * night;
        }
        if c[3] - v.cutoff < 0.0 {
            return None;
        }
        let alpha = ((c[0] + c[1] + c[2]) / 3.0).clamp(0.0, 1.0) * c[3];
        Some(([c[0], c[1], c[2]], alpha))
    }

    /// The console halo's values (logged by the client, M7g4).
    const HALO: ParticlesValues = ParticlesValues {
        color: [0.227_941_17, 0.911_764_7, 0.260_953_25, 1.0],
        strength: [1.0, 1.0, 1.0, 0.25],
        strength_night: [1.0, 1.0, 1.0, 0.25],
        main_st: [20.0, 2.0, 0.0, 0.0],
        main2_st: [100.0, 100.0, 0.0, 0.0],
        speed: [0.1, -0.2, 0.1, 0.1],
        fresnel_fade: 3.0,
        fresnel_pow: 1.0,
        cutoff: 0.0,
        fresnel_clip: true,
        mul_map: true,
        soft_edges: false,
        deform: false,
        refract: false,
        inv_fade: 0.0,
        deform_strength: 0.0,
        refract_strength: 0.0,
        deform_st: [1.0, 1.0, 0.0, 0.0],
        refract_st: [1.0, 1.0, 0.0, 0.0],
        speed2: [0.0; 4],
        textures: [None; PARTICLES_TEXTURES],
    };

    /// The door force field's values (`precursor_doorway_portal`, read
    /// with `sn-inspect prefab … --props`, M7g4).
    const PORTAL: ParticlesValues = ParticlesValues {
        color: [0.403_921_54, 1.0, 0.546_234, 1.0],
        strength: [4.0, 4.0, 4.0, 0.1],
        strength_night: [4.0, 4.0, 4.0, 0.1],
        main_st: [40.0, 40.0, 0.0, 0.0],
        main2_st: [0.1, 50.0, 0.5, 0.0],
        speed: [0.01, -0.05, -0.01, -2.0],
        fresnel_fade: 2.0,
        fresnel_pow: 5.0,
        cutoff: 0.01,
        fresnel_clip: false,
        mul_map: true,
        soft_edges: true,
        deform: true,
        refract: true,
        inv_fade: 1.6,
        deform_strength: 0.005,
        refract_strength: 0.02,
        deform_st: [0.2, 30.0, 0.0, 0.0],
        refract_st: [0.1, 60.0, 0.0, 0.0],
        speed2: [0.0, 0.5, 0.0, 0.5],
        textures: [None; PARTICLES_TEXTURES],
    };

    /// FX_SOFTEDGES: the factor on α where the mesh is `gap` metres in
    /// front of the scene.
    fn soft(v: &ParticlesValues, gap: f32) -> f32 {
        if v.soft_edges {
            (gap * v.inv_fade).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// FX_DEFORM: the uv every other texture is sampled at.
    fn deformed(v: &ParticlesValues, uv: [f32; 2], deform: [f32; 2]) -> [f32; 2] {
        if v.deform {
            [
                uv[0] + (deform[0] - 1.0) * v.deform_strength,
                uv[1] + (deform[1] - 1.0) * v.deform_strength,
            ]
        } else {
            uv
        }
    }

    /// FX_REFRACTMAP: the composite's uv offset (B.yz) from the refraction
    /// map's sample (r, g, a), the colour chain's α `a` and the fog's
    /// transmission `t` over the mesh's path through water.
    fn refraction(v: &ParticlesValues, r: f32, g: f32, alpha_tex: f32, a: f32, t: f32) -> [f32; 2] {
        if !v.refract {
            return [0.0; 2];
        }
        let k = v.refract_strength * a * t;
        [(alpha_tex * r * 2.0 - 1.0) * k, (g * 2.0 - 1.0) * k]
    }

    #[test]
    fn force_fields_follow_the_decoded_terms() {
        // Soft edges: gone where the field touches the scene, full 1/1.6 m
        // in front of it; off for the holograms.
        assert_eq!(soft(&PORTAL, 0.0), 0.0);
        assert!((soft(&PORTAL, 0.5) - 0.8).abs() < 1e-6);
        assert_eq!(soft(&PORTAL, 1.0), 1.0);
        assert_eq!(soft(&HALO, 0.0), 1.0);
        // Deform: a map value of 1 leaves the uv; 0 moves it by
        // -_DeformStrength (not centred on 0.5).
        assert_eq!(deformed(&PORTAL, [0.25, 0.75], [1.0, 1.0]), [0.25, 0.75]);
        let d = deformed(&PORTAL, [0.25, 0.75], [0.0, 0.5]);
        assert!((d[0] - 0.245).abs() < 1e-6 && (d[1] - 0.7475).abs() < 1e-6);
        assert_eq!(deformed(&HALO, [0.25, 0.75], [0.0, 0.0]), [0.25, 0.75]);
        // Refraction: a flat normal (0.5, 0.5 with a = 1) offsets nothing;
        // the offset scales with _RefractStrength, α and the transmission.
        assert_eq!(refraction(&PORTAL, 0.5, 0.5, 1.0, 0.3, 1.0), [0.0, 0.0]);
        let o = refraction(&PORTAL, 1.0, 0.0, 1.0, 0.5, 0.5);
        assert!((o[0] - 0.005).abs() < 1e-7 && (o[1] + 0.005).abs() < 1e-7);
        assert_eq!(refraction(&HALO, 1.0, 0.0, 1.0, 0.5, 1.0), [0.0, 0.0]);
    }

    #[test]
    fn particles_follow_the_decoded_terms() {
        let one = [1.0; 4];
        // Facing the camera: Fresnel 3 clamped to 1; α = 2 × _Color.a × 0.25
        // × mean brightness of 2 × _Color.rgb (clamped to 1).
        let (rgb, alpha) = particles(&HALO, 1.0, one, one, one, 0.0).unwrap();
        assert!((rgb[1] - 2.0 * 0.911_764_7).abs() < 1e-6);
        let mean = (2.0_f32 * (0.227_941_17 + 0.911_764_7 + 0.260_953_25) / 3.0).min(1.0);
        assert!((alpha - 0.5 * mean).abs() < 1e-6);
        // At the silhouette the halo is gone; a dark texel is transparent.
        assert_eq!(particles(&HALO, 0.0, one, one, one, 0.0).unwrap().1, 0.0);
        let dark = [0.0, 0.0, 0.0, 1.0];
        assert_eq!(particles(&HALO, 1.0, one, dark, one, 0.0).unwrap().1, 0.0);
        // A negative fade keeps the rim instead.
        let rim = ParticlesValues {
            fresnel_fade: -1.0,
            ..HALO
        };
        assert!(particles(&rim, 0.0, one, one, one, 0.0).unwrap().1 > 0.0);
        assert_eq!(particles(&rim, 1.0, one, one, one, 0.0).unwrap().1, 0.0);
        // _Cutoff discards below it; without FX_MULMAP the second texture
        // does not count.
        let cut = ParticlesValues {
            cutoff: 0.6,
            ..HALO
        };
        assert!(particles(&cut, 1.0, one, one, one, 0.0).is_none());
        let single = ParticlesValues {
            mul_map: false,
            ..HALO
        };
        let half = [0.5; 4];
        assert_eq!(
            particles(&single, 1.0, one, one, half, 0.0),
            particles(&single, 1.0, one, one, one, 0.0)
        );
        // Night strength by the local light.
        let night = ParticlesValues {
            strength_night: [0.0; 4],
            ..HALO
        };
        assert!(particles(&night, 1.0, one, one, one, 1.0).is_none_or(|(_, a)| a == 0.0));
    }

    #[test]
    fn vertices_are_interleaved_with_defaults() {
        let data = MeshData {
            positions: vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
            normals: Vec::new(),
            tangents: Vec::new(),
            colors: vec![[0.1, 0.2, 0.3, 0.4], [0.5, 0.6, 0.7, 0.8]],
            uvs: Vec::new(),
            indices: vec![0, 1, 0],
        };
        let bytes = vertex_bytes(&data);
        assert_eq!(bytes.len(), 2 * VERTEX_FLOATS * 4);
        let f: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        assert_eq!(
            &f[..12],
            &[1.0, 2.0, 3.0, 0.0, 1.0, 0.0, 0.1, 0.2, 0.3, 0.4, 0.0, 0.0]
        );
        assert_eq!(&f[12..15], &[4.0, 5.0, 6.0]);
        assert_eq!(&f[18..22], &[0.5, 0.6, 0.7, 0.8]);
    }
}
