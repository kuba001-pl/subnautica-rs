//! The sky (M8c2), ported from the game's uSky skybox and sky map shaders
//! (`docs/formats/sky.md`): drawn where the scene shows nothing, before the
//! water fog (the game draws its skybox before its fog image effect), and
//! rendered each frame into the 256² sky map the water surface reflects.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, texture_2d, texture_depth_2d,
    texture_depth_2d_multisampled, uniform_buffer,
};
use bevy::render::render_resource::{
    AddressMode, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, Extent3d, FilterMode, FragmentState,
    LoadOp, MipmapFilterMode, Operations, PipelineCache, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderStages, ShaderType, StoreOp, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor, UniformBuffer,
};
use bevy::render::render_resource::{
    BlendComponent, BlendFactor, BlendOperation, BlendState, Buffer, BufferInitDescriptor,
    BufferUsages, PrimitiveState, VertexState,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{
    ExtractedView, Msaa, ViewDepthTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms,
};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use sn_assets::SkyTextures;
use sn_unity::SkyManager;

use crate::sky::{SkyState, beta_r, linear};
use crate::stars::GpuStar;
use crate::textures::to_image;
use crate::water::{WaterFogPass, WaterWorld};
use crate::water_surface::WaterSurfacePass;

/// Size of the game's sky map.
const SKYMAP_SIZE: u32 = 256;
const SKYMAP_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;

/// Uniform of the sky; must match `SkyDome` in `sky_common.wgsl`.
#[derive(Resource, Clone, Copy, Debug, Default, ShaderType, ExtractResource)]
pub struct SkyDomeUniform {
    pub sun_dir: Vec4,
    pub correction: Vec4,
    pub beta_r: Vec4,
    pub beta_m: Vec4,
    pub mie_phase: Vec4,
    pub mie_const: Vec4,
    pub night_zenith: Vec4,
    pub sky_multiplier: Vec4,
    pub ground: Vec4,
    pub night_horizon: Vec4,
    pub moon_inner: Vec4,
    pub moon_outer: Vec4,
    pub moon_x: Vec4,
    pub moon_y: Vec4,
    pub moon_z: Vec4,
    pub planet_pos: Vec4,
    pub planet_rim: Vec4,
    pub planet_ambient: Vec4,
    pub planet_inner: Vec4,
    pub planet_outer: Vec4,
    pub misc: Vec4,
    pub clouds_x: Vec4,
    pub clouds_y: Vec4,
    pub clouds_z: Vec4,
    pub shade_sun: Vec4,
    pub shade_sky: Vec4,
    pub cloud_scatter: Vec4,
    pub secondary_dir: Vec4,
    pub secondary_colour: Vec4,
}

fn v4(c: [f32; 4]) -> Vec4 {
    Vec4::from_array(c)
}

/// `uSkyManager.SetConstantMaterialProperties` and
/// `SetVaryingMaterialProperties` (Unity coordinates, the scene's
/// settings: linear colour space, LDR sky, static night sky, no moon
/// light, the manager at the origin). `day`: game days since the start
/// (the planet's orbit); `seconds`: time since start-up (cloud rotation).
pub fn dome_uniform(
    m: &SkyManager,
    s: &SkyState,
    day: f64,
    seconds: f32,
    unit: f32,
) -> SkyDomeUniform {
    let d = &m.dome;
    let i = &s.dome;
    let t01 = s.timeline / 24.0;
    // `uSkyManager.BetaR` (our helper returns it × 1000).
    let beta = beta_r(m) / 1000.0;
    let beta_offset = beta * m.rayleigh_scattering.max(0.001);
    let beta_m = Vec3::splat(0.004 * 0.9);
    let g = m.sun_anisotropy;
    let hdr = d.linear_space && d.skybox_hdr;
    let mie_phase = Vec3::new(
        if hdr { 2.0 } else { 1.0 } * (1.0 - g * g) / (2.0 + g * g),
        1.0 + g * g,
        2.0 * g,
    );
    let mie_const = Vec3::new(1.0, beta.x / beta.y, beta.x / beta.z) * beta_m.x * m.mie_scattering;
    let ground_scale = if d.linear_space { 0.01 } else { 0.02 };
    let ground = beta_offset
        / (Vec3::new(m.ground_color[0], m.ground_color[1], m.ground_color[2]) * ground_scale);
    let correction = match (d.linear_space, d.skybox_hdr) {
        (true, true) => Vec2::new(0.38317, 1.413),
        (true, false) => Vec2::new(1.0, 2.0),
        _ => Vec2::ONE,
    };
    let night = i.night;
    // Static night sky: the coronas follow the night factor.
    let corona = night;
    let outer_power = if d.linear_space {
        if d.skybox_hdr { 16.0 } else { 12.0 }
    } else {
        8.0
    };
    let inner = |c: [f32; 4]| Vec4::new(c[0] * corona, c[1] * corona, c[2] * corona, 400.0 / c[3]);
    let outer = |c: [f32; 4]| {
        Vec4::new(
            c[0] * 0.25 * corona,
            c[1] * 0.25 * corona,
            c[2] * 0.25 * corona,
            outer_power / c[3],
        )
    };
    // No moon light in the scene: a fixed rotation.
    let moon = Mat3::from_quat(Quat::from_xyzw(
        -0.923_879_5,
        8.817_204e-8,
        8.817_204e-8,
        0.382_683_5,
    ));
    // The planet's orbit (degrees per game day) at its zenith angle.
    let orbit = (f64::from(d.planet_orbit_speed) * day).to_radians();
    let zenith = f64::from(d.planet_zenith).to_radians();
    let planet = Vec3::new(
        (zenith.sin() * orbit.cos()) as f32,
        zenith.cos() as f32,
        (zenith.sin() * orbit.sin()) as f32,
    ) * d.planet_distance;
    let eclipse = planet.normalize().dot(i.sun_dir).max(0.0).powf(50.0);
    let clouds = Mat3::from_quat(Quat::from_rotation_y(
        (d.clouds_rotate_speed * seconds).to_radians(),
    ));
    let shade = d
        .cloud_night_brightness
        .powf(if d.linear_space { 1.5 } else { 1.0 })
        .max(i.day)
        * m.exposure.sqrt();
    let night_horizon = v4(d.night_horizon_color) * night;
    let zenith_colour = Vec4::from_array(d.night_zenith_color.evaluate(t01)) * 0.01;
    let rim = linear(Vec3::new(
        d.planet_rim_color[0],
        d.planet_rim_color[1],
        d.planet_rim_color[2],
    ));
    let ambient = linear(Vec3::new(
        d.planet_ambient_light[0],
        d.planet_ambient_light[1],
        d.planet_ambient_light[2],
    ));
    SkyDomeUniform {
        sun_dir: i.sun_dir.extend(32.0 / m.sun_size),
        // The planet texture's size, for its level of detail.
        correction: Vec4::new(correction.x, correction.y, unit, 2048.0),
        beta_r: beta_offset.extend(0.0),
        beta_m: beta_m.extend(0.0),
        mie_phase: mie_phase.extend(0.0),
        mie_const: mie_const.extend(0.0),
        night_zenith: zenith_colour,
        sky_multiplier: Vec4::new(
            i.sunset,
            m.exposure * 4.0 * i.day * m.rayleigh_scattering.sqrt(),
            night,
            0.0,
        ),
        ground: ground.extend(0.0),
        night_horizon,
        moon_inner: inner(d.moon_inner_corona),
        moon_outer: outer(d.moon_outer_corona),
        moon_x: moon.row(0).extend(d.moon_size),
        moon_y: moon.row(1).extend(0.0),
        moon_z: moon.row(2).extend(0.0),
        planet_pos: planet.extend(d.planet_radius),
        planet_rim: rim.extend(0.0),
        planet_ambient: ambient.extend(d.planet_light_wrap),
        planet_inner: inner(d.planet_inner_corona),
        planet_outer: outer(d.planet_outer_corona),
        misc: Vec4::new(
            eclipse,
            d.clouds_attenuation,
            d.clouds_alpha_saturation,
            0.0,
        ),
        clouds_x: clouds.row(0).extend(0.0),
        clouds_y: clouds.row(1).extend(0.0),
        clouds_z: clouds.row(2).extend(0.0),
        shade_sun: (linear(i.light_colour) * shade).extend(d.sun_color_multiplier),
        shade_sky: (linear(i.sky_colour) * shade).extend(d.sky_color_multiplier),
        cloud_scatter: Vec4::new(
            d.clouds_scattering_exponent,
            d.clouds_scattering_multiplier,
            d.secondary_light_pow,
            0.0,
        ),
        // The secondary light (a story event's sun beam) is off.
        secondary_dir: Vec3::from_array(d.secondary_light_dir).extend(0.0),
        secondary_colour: Vec4::ZERO,
    }
}

/// Read at start-up; turned into GPU images by `create_images`.
#[derive(Resource)]
pub struct PendingSky(pub Option<SkyTextures>);

/// The game's sky settings and the game day (for the planet's orbit).
#[derive(Resource)]
pub struct SkyWorld {
    pub manager: SkyManager,
    pub day: f64,
}

/// uSky's star catalogue, read at start-up (`stars.rs`).
#[derive(Resource)]
pub struct PendingStars(pub Option<Vec<GpuStar>>);

#[derive(Resource, Clone, ExtractResource)]
struct StarCatalogue(Arc<Vec<GpuStar>>);

/// Uniform of the stars; must match `Stars` in `stars.wgsl`.
#[derive(Resource, Clone, Copy, Debug, Default, ShaderType, ExtractResource)]
pub struct StarsUniform {
    params: Vec4,
}

fn create_stars(mut commands: Commands, mut pending: ResMut<PendingStars>) {
    if let Some(stars) = pending.0.take() {
        info!("sky: {} stars", stars.len());
        commands.insert_resource(StarCatalogue(Arc::new(stars)));
    }
}

#[derive(Resource, Clone, ExtractResource)]
struct SkyImages {
    planet: Handle<Image>,
    burst: Handle<Image>,
    moon: Handle<Image>,
    clouds: Handle<Image>,
}

fn create_images(
    mut commands: Commands,
    mut pending: ResMut<PendingSky>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(t) = pending.0.take() else {
        return;
    };
    // The texture's own colour space: 1 = sRGB colour, 0 = linear data.
    let image = |tex: &sn_assets::TerrainTexture| to_image(tex, tex.texture.color_space == 1);
    let (Some(planet), Some(burst), Some(moon), Some(clouds)) = (
        image(&t.planet),
        image(&t.sun_burst),
        image(&t.moon),
        image(&t.clouds),
    ) else {
        error!("sky textures could not be decoded; no sky dome");
        return;
    };
    commands.insert_resource(SkyImages {
        planet: images.add(planet),
        burst: images.add(burst),
        moon: images.add(moon),
        clouds: images.add(clouds),
    });
}

fn update_sky(
    time: Res<Time>,
    world: Res<SkyWorld>,
    state: Res<SkyState>,
    water: Res<WaterWorld>,
    mut uniform: ResMut<SkyDomeUniform>,
    mut stars: ResMut<StarsUniform>,
) {
    // `uSkyManager`: stars are drawn while the sun is below 0.2, at
    // `StarIntensity × NightTime` (linear colour space).
    let d = &world.manager.dome;
    let brightness = if state.dome.sun_dir.y < 0.2 {
        d.star_intensity * state.dome.night
    } else {
        0.0
    };
    stars.params = Vec4::new(
        brightness,
        time.elapsed_secs() / 20.0,
        time.delta_secs(),
        crate::stars::QUAD_SIZE,
    );
    *uniform = dome_uniform(
        &world.manager,
        &state,
        world.day,
        time.elapsed_secs(),
        water.light_unit,
    );
}

pub struct SkyDomePlugin;

impl Plugin for SkyDomePlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "sky_common.wgsl");
        bevy::asset::embedded_asset!(app, "sky_dome.wgsl");
        bevy::asset::embedded_asset!(app, "stars.wgsl");
        app.add_plugins((
            ExtractResourcePlugin::<SkyDomeUniform>::default(),
            ExtractResourcePlugin::<SkyImages>::default(),
            ExtractResourcePlugin::<StarCatalogue>::default(),
            ExtractResourcePlugin::<StarsUniform>::default(),
        ))
        .init_resource::<SkyDomeUniform>()
        .init_resource::<StarsUniform>()
        .add_systems(Startup, create_images.run_if(resource_exists::<PendingSky>))
        .add_systems(
            Startup,
            create_stars.run_if(resource_exists::<PendingStars>),
        )
        .add_systems(
            Update,
            update_sky
                .run_if(resource_exists::<SkyWorld>)
                .run_if(resource_exists::<SkyState>)
                .run_if(resource_exists::<WaterWorld>),
        );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<StarsGpu>()
            .add_systems(RenderStartup, init_gpu)
            .add_systems(
                Render,
                (
                    prepare_pipelines.in_set(RenderSystems::Prepare),
                    prepare_uniform.in_set(RenderSystems::PrepareResources),
                ),
            )
            .add_systems(
                Core3d,
                (
                    sky_pass
                        .in_set(Core3dSystems::PostProcess)
                        .in_set(SkyPass)
                        .before(WaterFogPass),
                    // The game draws its stars in the transparent queue:
                    // after the fog, before the water.
                    stars_pass
                        .in_set(Core3dSystems::PostProcess)
                        .after(WaterFogPass)
                        .before(WaterSurfacePass),
                ),
            )
            .add_systems(
                Render,
                (
                    prepare_stars.in_set(RenderSystems::PrepareResources),
                    prepare_star_pipelines.in_set(RenderSystems::Prepare),
                ),
            );
    }
}

/// The stars' GPU state: pipelines, uniform, catalogue buffer.
#[derive(Resource, Default)]
struct StarsGpu {
    layouts: Option<[BindGroupLayoutDescriptor; 2]>,
    shader: Handle<Shader>,
    pipelines: HashMap<(TextureFormat, bool), CachedRenderPipelineId>,
    uniform: UniformBuffer<StarsUniform>,
    catalogue: Option<(Buffer, u32)>,
}

fn prepare_stars(
    mut gpu: ResMut<StarsGpu>,
    uniform: Option<Res<StarsUniform>>,
    catalogue: Option<Res<StarCatalogue>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    if let Some(u) = uniform {
        gpu.uniform.set(*u);
        gpu.uniform.write_buffer(&device, &queue);
    }
    if gpu.catalogue.is_none()
        && let Some(c) = catalogue
    {
        let bytes: Vec<u8> = c.0.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
        let buffer = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("star_catalogue"),
            contents: &bytes,
            usage: BufferUsages::STORAGE,
        });
        gpu.catalogue = Some((buffer, c.0.len() as u32));
    }
}

fn prepare_star_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    asset_server: Res<AssetServer>,
    mut gpu: ResMut<StarsGpu>,
    views: Query<(Entity, &ExtractedView, &Msaa), With<ViewTarget>>,
) {
    if gpu.layouts.is_none() {
        let layout = |multisampled: bool| {
            let depth = if multisampled {
                texture_depth_2d_multisampled()
            } else {
                texture_depth_2d()
            };
            let float = || texture_2d(TextureSampleType::Float { filterable: true });
            BindGroupLayoutDescriptor::new(
                "stars_layout",
                &BindGroupLayoutEntries::sequential(
                    ShaderStages::VERTEX_FRAGMENT,
                    (
                        uniform_buffer::<SkyDomeUniform>(false),
                        uniform_buffer::<ViewUniform>(true),
                        uniform_buffer::<StarsUniform>(false),
                        storage_buffer_read_only_sized(false, None),
                        float(),
                        float(),
                        sampler(SamplerBindingType::Filtering),
                        sampler(SamplerBindingType::Filtering),
                        depth,
                    ),
                ),
            )
        };
        gpu.layouts = Some([layout(false), layout(true)]);
        gpu.shader = asset_server.load("embedded://sn_client/stars.wgsl");
    }
    let Some(layouts) = gpu.layouts.clone() else {
        return;
    };
    for (entity, view, msaa) in &views {
        let multisampled = msaa.samples() > 1;
        let key = (view.target_format, multisampled);
        let id = match gpu.pipelines.get(&key) {
            Some(id) => *id,
            None => {
                let defs: Vec<_> = if multisampled {
                    vec!["MULTISAMPLED".into()]
                } else {
                    Vec::new()
                };
                // The game's state: Blend One OneMinusSrcAlpha with alpha 0,
                // i.e. added.
                let add = BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                };
                let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
                    label: Some("stars_pipeline".into()),
                    layout: vec![layouts[usize::from(multisampled)].clone()],
                    vertex: VertexState {
                        shader: gpu.shader.clone(),
                        shader_defs: defs.clone(),
                        entry_point: Some("vertex".into()),
                        buffers: Vec::new(),
                    },
                    primitive: PrimitiveState {
                        cull_mode: None,
                        ..default()
                    },
                    fragment: Some(FragmentState {
                        shader: gpu.shader.clone(),
                        shader_defs: defs,
                        entry_point: Some("fragment".into()),
                        targets: vec![Some(ColorTargetState {
                            format: view.target_format,
                            blend: Some(BlendState {
                                color: add,
                                alpha: add,
                            }),
                            write_mask: ColorWrites::ALL,
                        })],
                    }),
                    ..default()
                });
                gpu.pipelines.insert(key, id);
                id
            }
        };
        commands
            .entity(entity)
            .insert(StarsPipelineId(id, multisampled));
    }
}

#[derive(Component)]
struct StarsPipelineId(CachedRenderPipelineId, bool);

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // a render system
fn stars_pass(
    view: ViewQuery<(
        &ViewTarget,
        &ViewDepthTexture,
        &ViewUniformOffset,
        &StarsPipelineId,
    )>,
    cache: Res<PipelineCache>,
    gpu: Res<StarsGpu>,
    sky: Res<SkyPipelines>,
    map: Res<SkyMap>,
    images: Option<Res<SkyImages>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    view_uniforms: Res<ViewUniforms>,
    device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (target, depth, view_offset, id) = view.into_inner();
    let (Some(images), Some((catalogue, count)), Some(layouts)) =
        (images, gpu.catalogue.as_ref(), gpu.layouts.as_ref())
    else {
        return;
    };
    let (Some(moon), Some(clouds)) = (gpu_images.get(&images.moon), gpu_images.get(&images.clouds))
    else {
        return;
    };
    let Some(pipeline) = cache.get_render_pipeline(id.0) else {
        return;
    };
    let (Some(dome), Some(view_binding), Some(stars)) = (
        map.uniform.binding(),
        view_uniforms.uniforms.binding(),
        gpu.uniform.binding(),
    ) else {
        return;
    };
    let group = device.create_bind_group(
        "stars_bind_group",
        &cache.get_bind_group_layout(&layouts[usize::from(id.1)]),
        &BindGroupEntries::sequential((
            dome,
            view_binding,
            stars,
            catalogue.as_entire_binding(),
            &moon.texture_view,
            &clouds.texture_view,
            &sky.repeat,
            &sky.clamp,
            depth.view(),
        )),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(ctx.command_encoder(), "stars");
    {
        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("stars"),
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[view_offset.offset]);
        pass.draw(0..6, 0..*count);
    }
    span.end(ctx.command_encoder());
}

/// The sky pass (dome and sky map), for ordering other passes after it.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SkyPass;

/// The sky map with a view per mip, for the water surface.
#[derive(Resource)]
pub struct SkyMap {
    pub view: TextureView,
    pub sampler: Sampler,
    mips: Vec<TextureView>,
    uniform: UniformBuffer<SkyDomeUniform>,
    /// Whether the sky map has been drawn (the water reads it).
    pub ready: std::sync::atomic::AtomicBool,
}

#[derive(Resource)]
struct SkyPipelines {
    layouts: [BindGroupLayoutDescriptor; 2],
    repeat: Sampler,
    clamp: Sampler,
    shader: Handle<Shader>,
    fullscreen: FullscreenShader,
    /// (dome target format, multisampled) → (dome, sky map, downsample).
    pipelines: HashMap<(TextureFormat, bool), [CachedRenderPipelineId; 3]>,
}

fn init_gpu(
    mut commands: Commands,
    device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
) {
    let mips = SKYMAP_SIZE.ilog2() + 1;
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("sky_map"),
        size: Extent3d {
            width: SKYMAP_SIZE,
            height: SKYMAP_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: SKYMAP_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let linear_sampler = |address: AddressMode, label: &'static str| {
        device.create_sampler(&SamplerDescriptor {
            label: Some(label),
            address_mode_u: address,
            address_mode_v: address,
            address_mode_w: address,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Linear,
            ..default()
        })
    };
    commands.insert_resource(SkyMap {
        view: texture.create_view(&TextureViewDescriptor::default()),
        // The game's sky map: trilinear, clamped.
        sampler: linear_sampler(AddressMode::ClampToEdge, "sky_map_sampler"),
        mips: (0..mips)
            .map(|level| {
                texture.create_view(&TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..default()
                })
            })
            .collect(),
        uniform: UniformBuffer::default(),
        ready: std::sync::atomic::AtomicBool::new(false),
    });
    let layout = |multisampled: bool| {
        let depth = if multisampled {
            texture_depth_2d_multisampled()
        } else {
            texture_depth_2d()
        };
        let float = || texture_2d(TextureSampleType::Float { filterable: true });
        BindGroupLayoutDescriptor::new(
            "sky_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<SkyDomeUniform>(false),
                    float(),
                    float(),
                    float(),
                    float(),
                    sampler(SamplerBindingType::Filtering),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<ViewUniform>(true),
                    depth,
                    float(),
                ),
            ),
        )
    };
    commands.insert_resource(SkyPipelines {
        layouts: [layout(false), layout(true)],
        repeat: linear_sampler(AddressMode::Repeat, "sky_repeat"),
        clamp: linear_sampler(AddressMode::ClampToEdge, "sky_clamp"),
        shader: asset_server.load("embedded://sn_client/sky_dome.wgsl"),
        fullscreen: fullscreen.clone(),
        pipelines: HashMap::new(),
    });
}

#[derive(Component)]
struct SkyPipelineIds([CachedRenderPipelineId; 3], bool);

fn prepare_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SkyPipelines>,
    views: Query<(Entity, &ExtractedView, &Msaa), With<ViewTarget>>,
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
                let pipeline = |entry: &'static str, format: TextureFormat| {
                    cache.queue_render_pipeline(RenderPipelineDescriptor {
                        label: Some(format!("sky_{entry}_pipeline").into()),
                        layout: vec![pipelines.layouts[usize::from(multisampled)].clone()],
                        vertex: pipelines.fullscreen.to_vertex_state(),
                        fragment: Some(FragmentState {
                            shader: pipelines.shader.clone(),
                            shader_defs: defs.clone(),
                            entry_point: Some(entry.into()),
                            targets: vec![Some(ColorTargetState {
                                format,
                                blend: None,
                                write_mask: ColorWrites::ALL,
                            })],
                        }),
                        ..default()
                    })
                };
                let ids = [
                    pipeline("dome", view.target_format),
                    pipeline("skymap", SKYMAP_FORMAT),
                    pipeline("downsample", SKYMAP_FORMAT),
                ];
                pipelines.pipelines.insert(key, ids);
                ids
            }
        };
        commands
            .entity(entity)
            .insert(SkyPipelineIds(ids, multisampled));
    }
}

fn prepare_uniform(
    uniform: Option<Res<SkyDomeUniform>>,
    mut map: ResMut<SkyMap>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(uniform) = uniform else {
        return;
    };
    map.uniform.set(*uniform);
    map.uniform.write_buffer(&device, &queue);
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // a render system
fn sky_pass(
    view: ViewQuery<(
        &ViewTarget,
        &ViewDepthTexture,
        &ViewUniformOffset,
        &SkyPipelineIds,
    )>,
    cache: Res<PipelineCache>,
    pipelines: Res<SkyPipelines>,
    map: Res<SkyMap>,
    images: Option<Res<SkyImages>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    view_uniforms: Res<ViewUniforms>,
    device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (target, depth, view_offset, ids) = view.into_inner();
    let Some(images) = images else {
        return;
    };
    let (Some(planet), Some(burst), Some(moon), Some(clouds)) = (
        gpu_images.get(&images.planet),
        gpu_images.get(&images.burst),
        gpu_images.get(&images.moon),
        gpu_images.get(&images.clouds),
    ) else {
        return;
    };
    let [Some(dome), Some(skymap), Some(downsample)] =
        ids.0.map(|id| cache.get_render_pipeline(id))
    else {
        return;
    };
    let (Some(uniform), Some(view_binding)) =
        (map.uniform.binding(), view_uniforms.uniforms.binding())
    else {
        return;
    };
    let layout = cache.get_bind_group_layout(&pipelines.layouts[usize::from(ids.1)]);
    let group = |source: &TextureView| {
        device.create_bind_group(
            "sky_bind_group",
            &layout,
            &BindGroupEntries::sequential((
                uniform.clone(),
                &planet.texture_view,
                &burst.texture_view,
                &moon.texture_view,
                &clouds.texture_view,
                &pipelines.repeat,
                &pipelines.clamp,
                view_binding.clone(),
                depth.view(),
                source,
            )),
        )
    };
    // (label, target, pipeline, bind group): sky map, its mips, the dome.
    let mut passes = vec![("sky_map", &map.mips[0], skymap, group(&moon.texture_view))];
    for level in 1..map.mips.len() {
        passes.push((
            "sky_map_mip",
            &map.mips[level],
            downsample,
            group(&map.mips[level - 1]),
        ));
    }
    passes.push((
        "sky_dome",
        target.main_texture_view(),
        dome,
        group(&moon.texture_view),
    ));

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(ctx.command_encoder(), "sky");
    for (label, view, pipeline, group) in &passes {
        let load = if *label == "sky_dome" {
            LoadOp::Load
        } else {
            LoadOp::Clear(default())
        };
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
        pass.set_bind_group(0, group, &[view_offset.offset]);
        pass.draw(0..3, 0..1);
    }
    span.end(ctx.command_encoder());
    map.ready.store(true, std::sync::atomic::Ordering::Relaxed);
}
