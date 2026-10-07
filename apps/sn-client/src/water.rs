//! The underwater look (M8b): the game's water fog as a full-screen pass
//! over the lit HDR image (`water_fog.wgsl`, model in
//! `docs/formats/water.md`).
//!
//! The game reads its water settings at the camera (its sample distance
//! along the view ray is never set, so 0), from a volume around the camera
//! holding each 16 m cell's biome. Here the main world does the same on the
//! CPU every frame: the biome of the 8 cell centres around the camera,
//! blended trilinearly, into a uniform the pass reads.

use std::collections::HashMap;

use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
    UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId,
    ColorTargetState, ColorWrites, FragmentState, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler,
    SamplerBindingType, SamplerDescriptor, ShaderStages, ShaderType, TextureFormat,
    TextureSampleType,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::view::{
    ExtractedView, Msaa, ViewDepthTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms,
};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use sn_unity::{WaterSettings, WaterscapeVolume};

use crate::sky::SkyState;
use sn_world::{BatchCoord, BiomeMap, world_to_voxel};

/// Uniform of the fog pass; must match `WaterFog` in `water_common.wgsl`.
#[derive(Component, Clone, Copy, Debug, Default, ShaderType, ExtractComponent)]
pub struct WaterFog {
    pub extinction: Vec4,
    pub scattering: Vec4,
    pub emissive: Vec4,
    pub sun: Vec4,
    pub light: Vec4,
    pub phase: Vec4,
    pub sky: Vec4,
    pub misc: Vec4,
}

/// A biome's water as the fog pass uses it (the game's volume-texture
/// values; see `docs/formats/water.md` § Volume textures).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Coefficients {
    /// σt, start distance.
    extinction: Vec4,
    /// σs (scattering colour × scattering), sunlight scale × transmission.
    scattering: Vec4,
    /// Emissive, ambient scale.
    emissive: Vec4,
}

impl Coefficients {
    fn new(s: &WaterSettings, transmission: f32) -> Coefficients {
        let murk = s.murkiness / 100.0;
        let lin = |c: [f32; 4]| Color::srgba(c[0], c[1], c[2], c[3]).to_linear();
        let sc = lin(s.scattering_color);
        let em = lin(s.emissive);
        let scattering = s.scattering * murk;
        Coefficients {
            extinction: Vec4::new(
                (s.absorption[0] + s.scattering) * murk,
                (s.absorption[1] + s.scattering) * murk,
                (s.absorption[2] + s.scattering) * murk,
                s.start_distance,
            ),
            scattering: Vec4::new(
                sc.red * scattering,
                sc.green * scattering,
                sc.blue * scattering,
                s.sunlight_scale * transmission,
            ),
            emissive: Vec4::new(
                em.red * s.emissive_scale / 100.0,
                em.green * s.emissive_scale / 100.0,
                em.blue * s.emissive_scale / 100.0,
                s.ambient_scale,
            ),
        }
    }

    fn scaled(self, w: f32) -> Coefficients {
        Coefficients {
            extinction: self.extinction * w,
            scattering: self.scattering * w,
            emissive: self.emissive * w,
        }
    }

    fn add(self, o: Coefficients) -> Coefficients {
        Coefficients {
            extinction: self.extinction + o.extinction,
            scattering: self.scattering + o.scattering,
            emissive: self.emissive + o.emissive,
        }
    }
}

/// `WaterscapeVolume.Settings` defaults (the game's class), used for biomes
/// without settings (the open ocean, `void`).
fn default_settings() -> WaterSettings {
    WaterSettings {
        absorption: [100.0, 18.29155, 3.531373],
        scattering: 1.0,
        scattering_color: [1.0; 4],
        murkiness: 1.0,
        emissive: [0.0, 0.0, 0.0, 1.0],
        emissive_scale: 1.0,
        start_distance: 25.0,
        sunlight_scale: 1.0,
        ambient_scale: 1.0,
        temperature: 24.0,
    }
}

/// Everything needed to know the water at a point.
#[derive(Resource)]
pub struct WaterWorld {
    map: BiomeMap,
    names: Vec<String>,
    overrides: HashMap<BatchCoord, String>,
    /// Lower-case biome name → coefficients.
    biomes: HashMap<String, Coefficients>,
    fallback: Coefficients,
    pub volume: WaterscapeVolume,
    land_size: usize,
    batch_size: [i32; 3],
    /// Size of the settings cells (the game: 2 × region bounds / cells).
    cell: f32,
    /// One unit of the game's light, in our HDR units (see `unit`).
    pub light_unit: f32,
}

/// Water data read at start-up.
pub struct WaterData {
    pub map: BiomeMap,
    pub names: Vec<String>,
    pub overrides: HashMap<BatchCoord, String>,
    pub biomes: Vec<(String, WaterSettings)>,
    pub volume: WaterscapeVolume,
    pub cell: f32,
    pub land_size: usize,
    pub batch_size: [i32; 3],
}

impl WaterWorld {
    pub fn new(data: WaterData, light_unit: f32) -> WaterWorld {
        let t = data.volume.water_transmission.clamp(0.0, 1.0);
        WaterWorld {
            biomes: data
                .biomes
                .iter()
                .map(|(n, s)| (n.to_lowercase(), Coefficients::new(s, t)))
                .collect(),
            fallback: Coefficients::new(&default_settings(), t),
            map: data.map,
            names: data.names,
            overrides: data.overrides,
            volume: data.volume,
            land_size: data.land_size,
            batch_size: data.batch_size,
            cell: data.cell,
            light_unit,
        }
    }

    /// The biome name at a Unity world position (batch override, else map).
    pub fn biome_at(&self, p: [f32; 3]) -> Option<&str> {
        let v = world_to_voxel(p).map(|c| c.floor() as i32);
        let coord = BatchCoord::new(
            v[0].div_euclid(self.batch_size[0]),
            v[1].div_euclid(self.batch_size[1]),
            v[2].div_euclid(self.batch_size[2]),
        );
        if let Some(name) = self.overrides.get(&coord) {
            return Some(name);
        }
        let i = self.map.index_at(v[0], v[2], self.land_size)?;
        self.names.get(usize::from(i)).map(String::as_str)
    }

    fn coefficients_at(&self, p: [f32; 3]) -> Coefficients {
        self.biome_at(p)
            .and_then(|b| self.biomes.get(&b.to_lowercase()))
            .copied()
            .unwrap_or(self.fallback)
    }

    /// Settings at `p`, blended between the centres of the surrounding cells.
    fn blended_at(&self, p: [f32; 3]) -> Coefficients {
        let c = self.cell;
        let g = p.map(|x| x / c - 0.5);
        let base = g.map(f32::floor);
        let f = [0, 1, 2].map(|a| g[a] - base[a]);
        let mut sum = Coefficients::default();
        for corner in 0..8 {
            let o = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let w: f32 = (0..3)
                .map(|a| if o[a] == 1 { f[a] } else { 1.0 - f[a] })
                .product();
            if w == 0.0 {
                continue;
            }
            let centre = [0, 1, 2].map(|a| (base[a] + o[a] as f32 + 0.5) * c);
            sum = sum.add(self.coefficients_at(centre).scaled(w));
        }
        sum
    }
}

/// Our HDR value of a white Lambert surface lit by our sun (Unity's
/// lighting has no 1/π, so this is one unit of the game's light, assuming
/// the game's sun is one unit). Calibration: **hypothesis**.
pub fn unit(sun_lux: f32, exposure: f32) -> f32 {
    sun_lux / std::f32::consts::PI * exposure
}

/// `--no-water-fog`: the fog stays off (the surface still uses the uniform).
#[derive(Resource)]
pub struct WaterFogOff;

/// Every frame: the water at the camera into its fog uniform.
pub fn update_water_fog(
    water: Res<WaterWorld>,
    off: Option<Res<WaterFogOff>>,
    sky: Res<SkyState>,
    mut cameras: Query<(&Transform, &mut WaterFog)>,
) {
    // `WaterscapeVolume.PreRender`: `uSkyManager.GetLightDirection`, kept at
    // least slightly downwards (y only, not renormalised).
    let w = sky.to_sun_water;
    let to_sun = Vec3::new(w.x, w.y.max(0.01), w.z);
    let v = &water.volume;
    let g = v.scattering_phase;
    for (transform, mut fog) in &mut cameras {
        let p = transform.translation;
        let unity = [p.x, p.y, -p.z];
        let c = water.blended_at(unity);
        // Thicker water fog when the camera is just above the surface.
        let t = ((p.y - v.above_water_min_height)
            / (v.above_water_max_height - v.above_water_min_height))
            .clamp(0.0, 1.0);
        let scale = 1.0 + (v.above_water_density_scale - 1.0) * t;
        let unit = water.light_unit;
        let light = (sky.sun * v.sun_light_amount * v.water_transmission + sky.top_ambient) * unit;
        *fog = WaterFog {
            extinction: (c.extinction.truncate() * scale).extend(c.extinction.w),
            scattering: (c.scattering.truncate() * scale).extend(c.scattering.w),
            emissive: (c.emissive.truncate() * unit).extend(c.emissive.w),
            sun: to_sun.extend(v.sun_attenuation),
            light: light.extend(0.0),
            phase: Vec4::new(
                (1.0 - g * g) / (4.0 * std::f32::consts::PI),
                1.0 + g * g,
                2.0 * g,
                0.0,
            ),
            sky: (sky.fog_color * unit).extend(sky.fog_density),
            misc: Vec4::new(
                v.above_water_start_distance,
                v.water_offset,
                if off.is_some() { 0.0 } else { 1.0 },
                0.0,
            ),
        };
    }
}

/// The fog pass, for ordering other passes after it.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WaterFogPass;

pub struct WaterFogPlugin;

impl Plugin for WaterFogPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "water_common.wgsl");
        bevy::asset::embedded_asset!(app, "water_fog.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<WaterFog>::default(),
            UniformComponentPlugin::<WaterFog>::default(),
        ))
        .add_systems(
            Update,
            update_water_fog
                .run_if(resource_exists::<WaterWorld>)
                .run_if(resource_exists::<SkyState>),
        );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_pipelines.in_set(RenderSystems::Prepare))
            .add_systems(
                Core3d,
                water_fog_pass
                    .in_set(Core3dSystems::PostProcess)
                    .in_set(WaterFogPass)
                    .before(tonemapping),
            );
    }
}

#[derive(Resource)]
struct WaterFogPipeline {
    layouts: [BindGroupLayoutDescriptor; 2],
    sampler: Sampler,
    shader: Handle<Shader>,
    fullscreen: FullscreenShader,
    /// (target format, multisampled) → pipeline.
    pipelines: HashMap<(TextureFormat, bool), CachedRenderPipelineId>,
}

fn init_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
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
            "water_fog_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    depth,
                    uniform_buffer::<ViewUniform>(true),
                    uniform_buffer::<WaterFog>(true),
                ),
            ),
        )
    };
    commands.insert_resource(WaterFogPipeline {
        layouts: [layout(false), layout(true)],
        sampler: render_device.create_sampler(&SamplerDescriptor::default()),
        shader: asset_server.load("embedded://sn_client/water_fog.wgsl"),
        fullscreen: fullscreen.clone(),
        pipelines: HashMap::new(),
    });
}

#[derive(Component)]
struct WaterFogPipelineId(CachedRenderPipelineId, bool);

fn prepare_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut pipeline: ResMut<WaterFogPipeline>,
    views: Query<(Entity, &ExtractedView, &Msaa), With<WaterFog>>,
) {
    for (entity, view, msaa) in &views {
        let multisampled = msaa.samples() > 1;
        let key = (view.target_format, multisampled);
        let id = match pipeline.pipelines.get(&key) {
            Some(id) => *id,
            None => {
                let descriptor = RenderPipelineDescriptor {
                    label: Some("water_fog_pipeline".into()),
                    layout: vec![pipeline.layouts[usize::from(multisampled)].clone()],
                    vertex: pipeline.fullscreen.to_vertex_state(),
                    fragment: Some(FragmentState {
                        shader: pipeline.shader.clone(),
                        shader_defs: if multisampled {
                            vec!["MULTISAMPLED".into()]
                        } else {
                            Vec::new()
                        },
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
                pipeline.pipelines.insert(key, id);
                id
            }
        };
        commands
            .entity(entity)
            .insert(WaterFogPipelineId(id, multisampled));
    }
}

#[allow(clippy::too_many_arguments)] // a render system: one parameter per resource
fn water_fog_pass(
    view: ViewQuery<(
        &ViewTarget,
        &ViewDepthTexture,
        &ViewUniformOffset,
        &DynamicUniformIndex<WaterFog>,
        &WaterFogPipelineId,
    )>,
    pipeline_cache: Res<PipelineCache>,
    fog_pipeline: Res<WaterFogPipeline>,
    view_uniforms: Res<ViewUniforms>,
    fog_uniforms: Res<ComponentUniforms<WaterFog>>,
    render_device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (target, depth, view_offset, fog_index, pipeline_id) = view.into_inner();
    let Some(pipeline) = pipeline_cache.get_render_pipeline(pipeline_id.0) else {
        return;
    };
    let (Some(view_binding), Some(fog_binding)) = (
        view_uniforms.uniforms.binding(),
        fog_uniforms.uniforms().binding(),
    ) else {
        return;
    };
    let post = target.post_process_write();
    let layout = &fog_pipeline.layouts[usize::from(pipeline_id.1)];
    let bind_group = render_device.create_bind_group(
        "water_fog_bind_group",
        &pipeline_cache.get_bind_group_layout(layout),
        &BindGroupEntries::sequential((
            post.source,
            &fog_pipeline.sampler,
            depth.view(),
            view_binding,
            fog_binding,
        )),
    );
    let descriptor = RenderPassDescriptor {
        label: Some("water_fog_pass"),
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
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(ctx.command_encoder(), "water_fog");
    {
        let mut pass = ctx.command_encoder().begin_render_pass(&descriptor);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[view_offset.offset, fog_index.index()]);
        pass.draw(0..3, 0..1);
    }
    span.end(ctx.command_encoder());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coefficients_follow_the_game() {
        let s = WaterSettings {
            absorption: [125.0, 20.0, 4.0],
            scattering: 1.2,
            scattering_color: [1.0; 4],
            murkiness: 0.18,
            emissive: [0.0, 0.0, 0.0, 1.0],
            emissive_scale: 1.0,
            start_distance: 25.0,
            sunlight_scale: 1.0,
            ambient_scale: 1.5,
            temperature: 28.0,
        };
        let c = Coefficients::new(&s, 0.7);
        assert!((c.extinction.x - 126.2 * 0.0018).abs() < 1e-6);
        assert!((c.extinction.z - 5.2 * 0.0018).abs() < 1e-6);
        assert_eq!(c.extinction.w, 25.0);
        assert!((c.scattering.x - 1.2 * 0.0018).abs() < 1e-6);
        assert!((c.scattering.w - 0.7).abs() < 1e-6);
        assert_eq!(c.emissive.w, 1.5);
    }
}
