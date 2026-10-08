//! Light shafts under water (M8c5), ported from the game's
//! `WaterSunShaftsOnCamera` (`sun_shafts.wgsl`, `docs/formats/lighting.md`
//! § Light shafts): traced at half resolution, then added to the image after
//! the water surface (the game runs it as an image effect after its
//! transparent geometry).

use std::collections::HashMap;

use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, texture_2d_array, texture_depth_2d, texture_depth_2d_multisampled,
    uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BlendComponent,
    BlendFactor, BlendOperation, BlendState, CachedRenderPipelineId, ColorTargetState, ColorWrites,
    Extent3d, FilterMode, FragmentState, LoadOp, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler,
    SamplerBindingType, SamplerDescriptor, ShaderStages, ShaderType, StoreOp, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, UniformBuffer,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::{CachedTexture, FallbackImage, GpuImage, TextureCache};
use bevy::render::view::{
    ExtractedView, Msaa, ViewDepthTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms,
};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use crate::game_light::GameLightImages;
use crate::sky::{SkyState, light_rotation};
use crate::sky_dome::SkyWorld;
use crate::water::{WaterFog, WaterWorld};
use crate::water_surface::{WaterSurfacePass, WaterSurfaceWorld};

/// The scene's `WaterSunShaftsOnCamera` (read once from the main scene on the
/// dev machine; `docs/formats/lighting.md` § Light shafts).
const START_DISTANCE: f32 = 5.0;
const MAX_DISTANCE: f32 = 15.0;
const SHAFTS_SCALE: f32 = -0.16;
const INTENSITY: f32 = 3.93;
const TRACE_STEP: f32 = 0.05;
/// The shafts are traced at 1 / this resolution.
const REDUCTION: u32 = 2;
const SHAFTS_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// Uniform of the shafts; must match `SunShafts` in `sun_shafts.wgsl`.
#[derive(Resource, Clone, Copy, Debug, Default, ShaderType, ExtractResource)]
pub struct SunShaftsUniform {
    light_x: Vec4,
    light_y: Vec4,
    light_z: Vec4,
    params: Vec4,
    trace: Vec4,
    light: Vec4,
    colour_cast: Vec4,
}

fn update_shafts(
    time: Res<Time>,
    sky: Res<SkyState>,
    water: Res<WaterWorld>,
    sky_world: Option<Res<SkyWorld>>,
    surface: Option<Res<WaterSurfaceWorld>>,
    images: Option<Res<GameLightImages>>,
    mut uniform: ResMut<SunShaftsUniform>,
) {
    let has_caustics = images.as_deref().is_some_and(|i| i.has_caustics);
    let (frames, fps) = surface.as_deref().map_or((1, 25), |s| {
        (
            s.surface.num_caustics_frames.max(1),
            s.surface.caustics_frames_per_second.max(1),
        )
    });
    let frame =
        ((time.elapsed_secs_f64() * f64::from(fps)).floor() as i64).rem_euclid(i64::from(frames));
    // The sun light's local axes (`sunLight.transform.worldToLocalMatrix`; its
    // position only shifts the pattern), as rows over Bevy world coordinates.
    let rotation = sky_world
        .as_deref()
        .map_or(Quat::IDENTITY, |s| light_rotation(&s.manager, sky.timeline));
    let row = |axis: Vec3| {
        let a = rotation * axis;
        Vec4::new(a.x, a.y, -a.z, 0.0)
    };
    let v = &water.volume;
    *uniform = SunShaftsUniform {
        light_x: row(Vec3::X),
        light_y: row(Vec3::Y),
        light_z: row(Vec3::Z),
        params: Vec4::new(START_DISTANCE, MAX_DISTANCE, SHAFTS_SCALE, INTENSITY),
        trace: Vec4::new(
            TRACE_STEP,
            // The baked caustics are stored ÷ 6.
            6.0,
            frame as f32,
            if has_caustics { 1.0 } else { 0.0 },
        ),
        light: (sky.sun * water.light_unit).extend(0.0),
        colour_cast: Vec4::new(
            v.color_cast_distance_factor,
            v.color_cast_depth_factor,
            0.0,
            0.0,
        ),
    };
}

pub struct SunShaftsPlugin;

impl Plugin for SunShaftsPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "sun_shafts.wgsl");
        app.add_plugins(ExtractResourcePlugin::<SunShaftsUniform>::default())
            .init_resource::<SunShaftsUniform>()
            .add_systems(
                Update,
                update_shafts
                    .run_if(resource_exists::<SkyState>)
                    .run_if(resource_exists::<WaterWorld>),
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
                    prepare_uniform.in_set(RenderSystems::PrepareResources),
                    prepare_texture.in_set(RenderSystems::PrepareResources),
                ),
            )
            .add_systems(
                Core3d,
                sun_shafts_pass
                    .in_set(Core3dSystems::PostProcess)
                    .after(WaterSurfacePass)
                    .before(tonemapping),
            );
    }
}

#[derive(Resource)]
struct SunShaftsPipelines {
    layouts: [BindGroupLayoutDescriptor; 2],
    linear: Sampler,
    shader: Handle<Shader>,
    fullscreen: FullscreenShader,
    /// (target format, multisampled) → (trace, combine).
    pipelines: HashMap<(TextureFormat, bool), [CachedRenderPipelineId; 2]>,
    uniform: UniformBuffer<SunShaftsUniform>,
}

fn init_gpu(
    mut commands: Commands,
    device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
) {
    let layout = |multisampled: bool| {
        let depth = if multisampled {
            texture_depth_2d_multisampled()
        } else {
            texture_depth_2d()
        };
        BindGroupLayoutDescriptor::new(
            "sun_shafts_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<ViewUniform>(true),
                    uniform_buffer::<WaterFog>(true),
                    uniform_buffer::<SunShaftsUniform>(false),
                    depth,
                    texture_2d_array(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        )
    };
    commands.insert_resource(SunShaftsPipelines {
        layouts: [layout(false), layout(true)],
        linear: device.create_sampler(&SamplerDescriptor {
            label: Some("sun_shafts_linear"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        }),
        shader: asset_server.load("embedded://sn_client/sun_shafts.wgsl"),
        fullscreen: fullscreen.clone(),
        pipelines: HashMap::new(),
        uniform: UniformBuffer::default(),
    });
}

#[derive(Component)]
struct SunShaftsPipelineIds([CachedRenderPipelineId; 2], bool);

fn prepare_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SunShaftsPipelines>,
    views: Query<(Entity, &ExtractedView, &Msaa), With<DynamicUniformIndex<WaterFog>>>,
) {
    for (entity, view, msaa) in &views {
        let multisampled = msaa.samples() > 1;
        let key = (view.target_format, multisampled);
        let ids = match pipelines.pipelines.get(&key) {
            Some(ids) => *ids,
            None => {
                let defs = if multisampled {
                    vec!["MULTISAMPLED".into()]
                } else {
                    Vec::new()
                };
                let pipeline = |entry: &'static str, format, blend| {
                    cache.queue_render_pipeline(RenderPipelineDescriptor {
                        label: Some(format!("sun_shafts_{entry}_pipeline").into()),
                        layout: vec![pipelines.layouts[usize::from(multisampled)].clone()],
                        vertex: pipelines.fullscreen.to_vertex_state(),
                        fragment: Some(FragmentState {
                            shader: pipelines.shader.clone(),
                            shader_defs: defs.clone(),
                            entry_point: Some(entry.into()),
                            targets: vec![Some(ColorTargetState {
                                format,
                                blend,
                                write_mask: ColorWrites::ALL,
                            })],
                        }),
                        ..default()
                    })
                };
                let add = BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                };
                let ids = [
                    pipeline("trace", SHAFTS_FORMAT, None),
                    pipeline(
                        "combine",
                        view.target_format,
                        Some(BlendState {
                            color: add,
                            alpha: add,
                        }),
                    ),
                ];
                pipelines.pipelines.insert(key, ids);
                ids
            }
        };
        commands
            .entity(entity)
            .insert(SunShaftsPipelineIds(ids, multisampled));
    }
}

fn prepare_uniform(
    uniform: Option<Res<SunShaftsUniform>>,
    mut pipelines: ResMut<SunShaftsPipelines>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(uniform) = uniform else {
        return;
    };
    pipelines.uniform.set(*uniform);
    pipelines.uniform.write_buffer(&device, &queue);
}

#[derive(Component)]
struct ViewShaftsTexture(CachedTexture);

fn prepare_texture(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ExtractedView), With<DynamicUniformIndex<WaterFog>>>,
) {
    for (entity, view) in &views {
        let texture = cache.get(
            &device,
            TextureDescriptor {
                label: Some("sun_shafts"),
                size: Extent3d {
                    width: (view.viewport.z / REDUCTION).max(1),
                    height: (view.viewport.w / REDUCTION).max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: SHAFTS_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );
        commands.entity(entity).insert(ViewShaftsTexture(texture));
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // a render system
fn sun_shafts_pass(
    view: ViewQuery<(
        &ViewTarget,
        &ViewDepthTexture,
        &ViewUniformOffset,
        &DynamicUniformIndex<WaterFog>,
        &SunShaftsPipelineIds,
        &ViewShaftsTexture,
    )>,
    cache: Res<PipelineCache>,
    pipelines: Res<SunShaftsPipelines>,
    images: Option<Res<GameLightImages>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    view_uniforms: Res<ViewUniforms>,
    fog_uniforms: Res<ComponentUniforms<WaterFog>>,
    device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (target, depth, view_offset, fog_index, ids, shafts) = view.into_inner();
    let Some(caustics) = images.and_then(|i| gpu_images.get(&i.caustics)) else {
        return;
    };
    let [Some(trace), Some(combine)] = ids.0.map(|id| cache.get_render_pipeline(id)) else {
        return;
    };
    let (Some(view_binding), Some(fog_binding), Some(uniform)) = (
        view_uniforms.uniforms.binding(),
        fog_uniforms.uniforms().binding(),
        pipelines.uniform.binding(),
    ) else {
        return;
    };
    let layout = cache.get_bind_group_layout(&pipelines.layouts[usize::from(ids.1)]);
    // The trace pass binds a placeholder in the shafts slot (it writes them).
    let group = |shafts_view| {
        device.create_bind_group(
            "sun_shafts_bind_group",
            &layout,
            &BindGroupEntries::sequential((
                view_binding.clone(),
                fog_binding.clone(),
                uniform.clone(),
                depth.view(),
                &caustics.texture_view,
                &caustics.sampler,
                shafts_view,
                &pipelines.linear,
            )),
        )
    };
    let trace_group = group(&fallback.d2.texture_view);
    let combine_group = group(&shafts.0.default_view);
    let passes = [
        (
            "sun_shafts_trace",
            &shafts.0.default_view,
            LoadOp::Clear(default()),
            trace,
            &trace_group,
        ),
        (
            "sun_shafts_combine",
            target.main_texture_view(),
            LoadOp::Load,
            combine,
            &combine_group,
        ),
    ];
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(ctx.command_encoder(), "sun_shafts");
    for (label, view, load, pipeline, group) in passes {
        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[view_offset.offset, fog_index.index()]);
        pass.draw(0..3, 0..1);
    }
    span.end(ctx.command_encoder());
}
