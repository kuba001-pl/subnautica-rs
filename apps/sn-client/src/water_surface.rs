//! The ocean surface (M8c1), ported from the game's `WaterSurface`
//! ("Medium" water quality: baked waves). See `docs/formats/water.md`
//! § Water surface.
//!
//! Every frame, as the game does: two of the 64 baked wave frames are
//! blended into a 512² displacement map, its normal map (with mips) is
//! derived from it, and foam accumulates where the waves squeeze together.
//! Then, after the fog pass (the game draws its water after its fog image
//! effect), the surface is drawn over the fogged image, which it reads for
//! refraction, together with the depth buffer.
//!
//! The mesh is ours (the game refines patches by screen-space error): a
//! fine grid near the camera, a coarser one out to 208 m (the waves have
//! faded out by 200 m) and a flat ring to the horizon, all made in the
//! vertex shader.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
    UniformComponentPlugin,
};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, texture_2d_array, texture_depth_2d, texture_depth_2d_multisampled,
    uniform_buffer,
};
use bevy::render::render_resource::{
    AddressMode, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
    BlendComponent, BlendFactor, BlendOperation, BlendState, CachedRenderPipelineId,
    ColorTargetState, ColorWrites, CompareFunction, DepthStencilState, Extent3d, FilterMode,
    FragmentState, LoadOp, MipmapFilterMode, Operations, PipelineCache, PrimitiveState,
    RenderPassColorAttachment, RenderPassDepthStencilAttachment, RenderPassDescriptor,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
    ShaderType, StoreOp, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension,
    UniformBuffer, VertexState,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::{CachedTexture, GpuImage, TextureCache};
use bevy::render::view::{
    ExtractedView, Msaa, ViewDepthTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms,
};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use sn_assets::WaterSurfaceData;
use sn_unity::WaterSurface;

use crate::sky::SkyState;
use crate::sky_dome::{SkyMap, SkyWorld};
use crate::textures::{linear, to_image};
use crate::water::{WaterFog, WaterFogPass, WaterWorld};

/// Size of the game's displacement, normal and foam maps.
const MAP_SIZE: u32 = 512;
/// The game's displacement map format is RGBA float; half floats are
/// filterable everywhere and keep ±300 cm to a fraction of a millimetre.
const DISPLACEMENT_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
const NORMALS_FORMAT: TextureFormat = TextureFormat::Rg16Float;
const FOAM_FORMAT: TextureFormat = TextureFormat::R16Float;
const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;
/// Vertices per draw (6 per quad) of the grid parts in `water_surface.wgsl`.
const INNER_VERTICES: u32 = 256 * 256 * 6;
const OUTER_VERTICES: u32 = 416 * 416 * 6;
const FAR_VERTICES: u32 = 9 * 6;

/// Read at start-up; turned into GPU images by `create_images`.
#[derive(Resource)]
pub struct PendingWaterSurface(pub Option<WaterSurfaceData>);

/// The scene's surface settings and the images made from the game's
/// textures.
#[derive(Resource)]
pub struct WaterSurfaceWorld {
    pub surface: WaterSurface,
    frame_count: u32,
}

#[derive(Resource, Clone, ExtractResource)]
struct WaterSurfaceImages {
    frames: Handle<Image>,
    foam: Handle<Image>,
    foam_mask: Handle<Image>,
}

/// Per-frame values of the wave and foam passes; must match `WaterSim` in
/// `water_sim.wgsl`.
#[derive(Resource, Clone, Copy, Default, ShaderType, ExtractResource)]
struct WaterSim {
    frames: Vec4,
    foam: Vec4,
}

/// The game's surface clock (`WaterSurface.time`).
#[derive(Resource, Default)]
struct SurfaceClock(f64);

/// Uniform of the surface; must match `WaterSurface` in
/// `water_surface.wgsl`.
#[derive(Component, Clone, Copy, Debug, Default, ShaderType, ExtractComponent)]
pub struct WaterSurfaceUniform {
    pub grid: Vec4,
    pub optics: Vec4,
    pub reflection: Vec4,
    pub refraction: Vec4,
    pub back_light: Vec4,
    pub sun: Vec4,
    pub ambient: Vec4,
    pub mean_sky: Vec4,
    pub foam: Vec4,
}

/// The 64 frames as one array texture (RGBA8, stored linear), the foam
/// textures as they are (sRGB, with mips).
fn create_images(
    mut commands: Commands,
    mut pending: ResMut<PendingWaterSurface>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(data) = pending.0.take() else {
        return;
    };
    let first = &data.frames[0].texture;
    let (w, h) = (first.width as u32, first.height as u32);
    let mut bytes = Vec::with_capacity((w * h * 4) as usize * data.frames.len());
    for f in &data.frames {
        if (f.texture.width as u32, f.texture.height as u32) != (w, h) {
            error!(
                "water frame {} has another size; no water surface",
                f.texture.name
            );
            return;
        }
        match f.texture.decode_rgba(&f.data) {
            Ok(rgba) => bytes.extend_from_slice(&rgba),
            Err(e) => {
                error!("water frame {}: {e}; no water surface", f.texture.name);
                return;
            }
        }
    }
    let mut frames = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: data.frames.len() as u32,
        },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    frames.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    let (Some(foam), Some(foam_mask)) =
        (to_image(&data.foam, true), to_image(&data.foam_mask, true))
    else {
        error!("water foam textures could not be decoded; no water surface");
        return;
    };
    info!(
        "water surface: {} wave frames of {w}×{h}, foam {}×{}, patch {} m, {} s per cycle",
        data.frames.len(),
        data.foam.texture.width,
        data.foam.texture.height,
        data.surface.patch_length / 100.0,
        data.surface.sequence_length
    );
    commands.insert_resource(WaterSurfaceImages {
        frames: images.add(frames),
        foam: images.add(foam),
        foam_mask: images.add(foam_mask),
    });
    commands.insert_resource(WaterSurfaceWorld {
        frame_count: data.frames.len() as u32,
        surface: data.surface,
    });
}

/// Which frames to blend and the foam rates, from the surface clock (the
/// game: `WaterSurface.DoUpdate` and `UpdateFoamTexture`).
fn update_water_sim(
    time: Res<Time>,
    world: Res<WaterSurfaceWorld>,
    mut clock: ResMut<SurfaceClock>,
    mut sim: ResMut<WaterSim>,
) {
    let s = &world.surface;
    let dt = time.delta_secs() * s.time_scale;
    clock.0 += f64::from(dt);
    let count = world.frame_count;
    let position = clock.0 * f64::from(count) / f64::from(s.sequence_length.max(1e-3));
    let first = position.floor();
    let a = (first as u64 % u64::from(count)) as f32;
    let b = ((first as u64 + 1) % u64::from(count)) as f32;
    *sim = WaterSim {
        frames: Vec4::new(a, b, (position - first) as f32, 0.0),
        foam: Vec4::new(
            s.patch_length,
            1.0 / MAP_SIZE as f32,
            (1.0 - dt * s.foam_decay).max(0.0),
            s.foam_rate * dt / 0.008333,
        ),
    };
}

/// The surface's uniform for each camera (the game:
/// `SetupSurfaceMaterialConstantValues` and `SetupSurfaceMaterial`).
fn update_water_surface(
    world: Res<WaterSurfaceWorld>,
    water: Res<WaterWorld>,
    sky: Res<SkyState>,
    sky_world: Option<Res<SkyWorld>>,
    mut cameras: Query<(&Transform, &mut WaterSurfaceUniform)>,
) {
    // The sky map is in game units; 0 = no sky map (mean sky colour).
    let sky_map_unit = if sky_world.is_some() {
        water.light_unit
    } else {
        0.0
    };
    let s = &world.surface;
    let unit = water.light_unit;
    let level = water.volume.water_offset + s.water_offset;
    let transmission = water.volume.water_transmission;
    let r0 = ((1.0 - s.refraction_index) / (1.0 + s.refraction_index)).powi(2);
    let back = linear(s.back_light_tint).truncate() * transmission;
    for (transform, mut uniform) in &mut cameras {
        let p = transform.translation;
        let depth = (level - p.y).max(0.0);
        let brightness = if s.use_under_water_brightness_curve {
            s.under_water_brightness_curve.evaluate(depth)
        } else {
            1.0
        };
        *uniform = WaterSurfaceUniform {
            // Snapped to the coarse grid's 1 m so vertices stay put.
            grid: Vec4::new(p.x.floor(), p.z.floor(), level, s.patch_length),
            optics: Vec4::new(
                s.patch_length / MAP_SIZE as f32 * 2.0,
                r0,
                1.0 / s.screen_space_refraction_index,
                s.under_water_refraction_index + depth * s.under_water_refraction_depth_scale,
            ),
            reflection: linear(s.reflection_color).truncate().extend(sky_map_unit),
            refraction: linear(s.refraction_color).truncate().extend(0.0),
            back_light: back.extend(s.sun_reflection_gloss),
            sun: (sky.sun * unit).extend(s.sun_reflection_amount),
            ambient: (sky.top_ambient * unit).extend(s.foam_smoothing),
            mean_sky: (sky.mean_sky * unit).extend(brightness),
            foam: Vec4::new(
                s.foam_scale,
                s.foam_distance,
                s.sub_surface_foam_scale,
                s.displacement_texture_foam_amount_multiplier,
            ),
        };
    }
}

pub struct WaterSurfacePlugin;

impl Plugin for WaterSurfacePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "water_sim.wgsl");
        bevy::asset::embedded_asset!(app, "water_surface.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<WaterSurfaceUniform>::default(),
            UniformComponentPlugin::<WaterSurfaceUniform>::default(),
            ExtractResourcePlugin::<WaterSurfaceImages>::default(),
            ExtractResourcePlugin::<WaterSim>::default(),
        ))
        .init_resource::<SurfaceClock>()
        .init_resource::<WaterSim>()
        .add_systems(
            Startup,
            create_images.run_if(resource_exists::<PendingWaterSurface>),
        )
        .add_systems(
            Update,
            (
                update_water_sim,
                update_water_surface
                    .run_if(resource_exists::<WaterWorld>)
                    .run_if(resource_exists::<SkyState>),
            )
                .run_if(resource_exists::<WaterSurfaceWorld>),
        );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_gpu)
            .add_systems(
                Render,
                (
                    prepare_pipelines.in_set(RenderSystems::Prepare),
                    prepare_sim_uniform.in_set(RenderSystems::PrepareResources),
                    prepare_water_depth.in_set(RenderSystems::PrepareResources),
                ),
            )
            .add_systems(
                Core3d,
                water_surface_pass
                    .in_set(Core3dSystems::PostProcess)
                    .after(WaterFogPass)
                    .before(tonemapping),
            );
    }
}

/// The displacement, normal (with a view per mip) and foam maps.
#[derive(Resource)]
struct WaterMaps {
    displacement: TextureView,
    normals: TextureView,
    normal_mips: Vec<TextureView>,
    foam: TextureView,
    /// The foam map starts cleared (the game clears it once).
    foam_cleared: AtomicBool,
    sim_uniform: UniformBuffer<WaterSim>,
}

#[derive(Resource)]
struct WaterSurfacePipelines {
    sim_layout: BindGroupLayoutDescriptor,
    surface_layouts: [BindGroupLayoutDescriptor; 2],
    repeat_sampler: Sampler,
    clamp_sampler: Sampler,
    sim_shader: Handle<Shader>,
    surface_shader: Handle<Shader>,
    fullscreen: FullscreenShader,
    /// interpolate, normals, downsample, foam.
    sim: Option<[CachedRenderPipelineId; 4]>,
    /// (target format, multisampled) → pipeline.
    surface: HashMap<(TextureFormat, bool), CachedRenderPipelineId>,
}

fn map(device: &RenderDevice, label: &'static str, format: TextureFormat, mips: u32) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d {
            width: MAP_SIZE,
            height: MAP_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn init_gpu(
    mut commands: Commands,
    device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
) {
    let mips = MAP_SIZE.ilog2() + 1;
    let displacement = map(&device, "water_displacement", DISPLACEMENT_FORMAT, 1);
    let normals = map(&device, "water_normals", NORMALS_FORMAT, mips);
    let foam = map(&device, "water_foam", FOAM_FORMAT, 1);
    commands.insert_resource(WaterMaps {
        displacement: displacement.create_view(&TextureViewDescriptor::default()),
        normals: normals.create_view(&TextureViewDescriptor::default()),
        normal_mips: (0..mips)
            .map(|level| {
                normals.create_view(&TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..default()
                })
            })
            .collect(),
        foam: foam.create_view(&TextureViewDescriptor::default()),
        foam_cleared: AtomicBool::new(false),
        sim_uniform: UniformBuffer::default(),
    });

    let sim_layout = BindGroupLayoutDescriptor::new(
        "water_sim_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d_array(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<WaterSim>(false),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let surface_layout = |multisampled: bool| {
        let depth = if multisampled {
            texture_depth_2d_multisampled()
        } else {
            texture_depth_2d()
        };
        let float = || texture_2d(TextureSampleType::Float { filterable: true });
        BindGroupLayoutDescriptor::new(
            "water_surface_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    uniform_buffer::<ViewUniform>(true),
                    uniform_buffer::<WaterFog>(true),
                    uniform_buffer::<WaterSurfaceUniform>(true),
                    float(),
                    float(),
                    float(),
                    float(),
                    float(),
                    float(),
                    depth,
                    sampler(SamplerBindingType::Filtering),
                    sampler(SamplerBindingType::Filtering),
                    float(),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        )
    };
    let repeat_sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("water_repeat"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        address_mode_w: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        // The game's normal map: anisotropic level 8.
        anisotropy_clamp: 8,
        ..default()
    });
    let clamp_sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("water_clamp"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    commands.insert_resource(WaterSurfacePipelines {
        sim_layout,
        surface_layouts: [surface_layout(false), surface_layout(true)],
        repeat_sampler,
        clamp_sampler,
        sim_shader: asset_server.load("embedded://sn_client/water_sim.wgsl"),
        surface_shader: asset_server.load("embedded://sn_client/water_surface.wgsl"),
        fullscreen: fullscreen.clone(),
        sim: None,
        surface: HashMap::new(),
    });
}

#[derive(Component)]
struct WaterSurfacePipelineId(CachedRenderPipelineId, bool);

fn prepare_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut pipelines: ResMut<WaterSurfacePipelines>,
    views: Query<(Entity, &ExtractedView, &Msaa), With<WaterSurfaceUniform>>,
) {
    if pipelines.sim.is_none() {
        let sim = |entry: &'static str, format: TextureFormat, blend: Option<BlendState>| {
            pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some(format!("water_{entry}_pipeline").into()),
                layout: vec![pipelines.sim_layout.clone()],
                vertex: pipelines.fullscreen.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: pipelines.sim_shader.clone(),
                    entry_point: Some(entry.into()),
                    targets: vec![Some(ColorTargetState {
                        format,
                        blend,
                        write_mask: ColorWrites::ALL,
                    })],
                    ..default()
                }),
                ..default()
            })
        };
        // The game's foam pass: Blend One SrcAlpha (new + old × decay).
        let foam_blend = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::SrcAlpha,
            operation: BlendOperation::Add,
        };
        pipelines.sim = Some([
            sim("interpolate", DISPLACEMENT_FORMAT, None),
            sim("normals", NORMALS_FORMAT, None),
            sim("downsample", NORMALS_FORMAT, None),
            sim(
                "foam",
                FOAM_FORMAT,
                Some(BlendState {
                    color: foam_blend,
                    alpha: foam_blend,
                }),
            ),
        ]);
    }
    for (entity, view, msaa) in &views {
        let multisampled = msaa.samples() > 1;
        let key = (view.target_format, multisampled);
        let id = match pipelines.surface.get(&key) {
            Some(id) => *id,
            None => {
                let shader_defs = if multisampled {
                    vec!["MULTISAMPLED".into()]
                } else {
                    Vec::new()
                };
                let descriptor = RenderPipelineDescriptor {
                    label: Some("water_surface_pipeline".into()),
                    layout: vec![pipelines.surface_layouts[usize::from(multisampled)].clone()],
                    vertex: VertexState {
                        shader: pipelines.surface_shader.clone(),
                        shader_defs: shader_defs.clone(),
                        entry_point: Some("vertex".into()),
                        buffers: Vec::new(),
                    },
                    // Seen from both sides, as in the game (Cull Off).
                    primitive: PrimitiveState {
                        cull_mode: None,
                        ..default()
                    },
                    // Our own depth buffer, so waves hide each other; the
                    // scene's depth is tested in the shader.
                    depth_stencil: Some(DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(CompareFunction::GreaterEqual),
                        stencil: default(),
                        bias: default(),
                    }),
                    fragment: Some(FragmentState {
                        shader: pipelines.surface_shader.clone(),
                        shader_defs,
                        entry_point: Some("fragment".into()),
                        targets: vec![Some(ColorTargetState {
                            format: view.target_format,
                            blend: None,
                            write_mask: ColorWrites::ALL,
                        })],
                    }),
                    ..default()
                };
                let id = pipeline_cache.queue_render_pipeline(descriptor);
                pipelines.surface.insert(key, id);
                id
            }
        };
        commands
            .entity(entity)
            .insert(WaterSurfacePipelineId(id, multisampled));
    }
}

fn prepare_sim_uniform(
    sim: Option<Res<WaterSim>>,
    mut maps: ResMut<WaterMaps>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(sim) = sim else {
        return;
    };
    maps.sim_uniform.set(*sim);
    maps.sim_uniform.write_buffer(&device, &queue);
}

#[derive(Component)]
struct ViewWaterDepth(CachedTexture);

fn prepare_water_depth(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ExtractedView), With<WaterSurfaceUniform>>,
) {
    for (entity, view) in &views {
        let texture = cache.get(
            &device,
            TextureDescriptor {
                label: Some("water_surface_depth"),
                size: Extent3d {
                    width: view.viewport.z.max(1),
                    height: view.viewport.w.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
        );
        commands.entity(entity).insert(ViewWaterDepth(texture));
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // a render system
fn water_surface_pass(
    view: ViewQuery<(
        &ViewTarget,
        &ViewDepthTexture,
        &ViewUniformOffset,
        &DynamicUniformIndex<WaterFog>,
        &DynamicUniformIndex<WaterSurfaceUniform>,
        &WaterSurfacePipelineId,
        &ViewWaterDepth,
    )>,
    pipeline_cache: Res<PipelineCache>,
    pipelines: Res<WaterSurfacePipelines>,
    maps: Res<WaterMaps>,
    sky_map: Res<SkyMap>,
    images: Option<Res<WaterSurfaceImages>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    view_uniforms: Res<ViewUniforms>,
    fog_uniforms: Res<ComponentUniforms<WaterFog>>,
    surface_uniforms: Res<ComponentUniforms<WaterSurfaceUniform>>,
    device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (target, depth, view_offset, fog_index, surface_index, pipeline_id, water_depth) =
        view.into_inner();
    let Some(images) = images else {
        return;
    };
    let (Some(frames), Some(foam_tex), Some(foam_mask)) = (
        gpu_images.get(&images.frames),
        gpu_images.get(&images.foam),
        gpu_images.get(&images.foam_mask),
    ) else {
        return;
    };
    let Some(sim_ids) = pipelines.sim else {
        return;
    };
    let sim_pipelines = sim_ids.map(|id| pipeline_cache.get_render_pipeline(id));
    let [
        Some(interpolate),
        Some(normals),
        Some(downsample),
        Some(foam),
    ] = sim_pipelines
    else {
        return;
    };
    let Some(surface_pipeline) = pipeline_cache.get_render_pipeline(pipeline_id.0) else {
        return;
    };
    let (Some(view_binding), Some(fog_binding), Some(surface_binding), Some(sim_binding)) = (
        view_uniforms.uniforms.binding(),
        fog_uniforms.uniforms().binding(),
        surface_uniforms.uniforms().binding(),
        maps.sim_uniform.binding(),
    ) else {
        return;
    };

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();

    // 1. The wave maps.
    let sim_layout = pipeline_cache.get_bind_group_layout(&pipelines.sim_layout);
    let sim_group = |source: &TextureView| {
        device.create_bind_group(
            "water_sim_bind_group",
            &sim_layout,
            &BindGroupEntries::sequential((
                &frames.texture_view,
                &pipelines.repeat_sampler,
                sim_binding.clone(),
                source,
                &pipelines.repeat_sampler,
            )),
        )
    };
    let clear = LoadOp::Clear(default());
    let foam_load = if maps.foam_cleared.swap(true, Ordering::Relaxed) {
        LoadOp::Load
    } else {
        clear
    };
    // (label, target, load, pipeline, bind group), in order.
    let mut passes = vec![
        (
            "water_interpolate",
            &maps.displacement,
            clear,
            interpolate,
            sim_group(&maps.foam),
        ),
        (
            "water_normals",
            &maps.normal_mips[0],
            clear,
            normals,
            sim_group(&maps.displacement),
        ),
    ];
    for level in 1..maps.normal_mips.len() {
        passes.push((
            "water_normals_mip",
            &maps.normal_mips[level],
            clear,
            downsample,
            sim_group(&maps.normal_mips[level - 1]),
        ));
    }
    passes.push((
        "water_foam",
        &maps.foam,
        foam_load,
        foam,
        sim_group(&maps.displacement),
    ));
    let span = diagnostics.time_span(ctx.command_encoder(), "water_sim");
    for (label, target, load, pipeline, group) in &passes {
        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: *load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
    span.end(ctx.command_encoder());

    // 2. The surface over a copy of the fogged image.
    let post = target.post_process_write();
    let layout = &pipelines.surface_layouts[usize::from(pipeline_id.1)];
    let bind_group = device.create_bind_group(
        "water_surface_bind_group",
        &pipeline_cache.get_bind_group_layout(layout),
        &BindGroupEntries::sequential((
            view_binding,
            fog_binding,
            surface_binding,
            &maps.displacement,
            &maps.normals,
            &maps.foam,
            &foam_tex.texture_view,
            &foam_mask.texture_view,
            post.source,
            depth.view(),
            &pipelines.repeat_sampler,
            &pipelines.clamp_sampler,
            &sky_map.view,
            &sky_map.sampler,
        )),
    );
    let span = diagnostics.time_span(ctx.command_encoder(), "water_surface");
    ctx.command_encoder().copy_texture_to_texture(
        post.source_texture.as_image_copy(),
        post.destination_texture.as_image_copy(),
        post.source_texture.size(),
    );
    {
        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("water_surface_pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: post.destination,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &water_depth.0.default_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.0),
                        store: StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        pass.set_pipeline(surface_pipeline);
        pass.set_bind_group(
            0,
            &bind_group,
            &[view_offset.offset, fog_index.index(), surface_index.index()],
        );
        pass.draw(0..INNER_VERTICES, 0..1);
        pass.draw(0..OUTER_VERTICES, 1..2);
        pass.draw(0..FAR_VERTICES, 2..3);
    }
    span.end(ctx.command_encoder());
}
