//! The game's lighting of opaque surfaces (M8c3): `game_light.wgsl` ports the
//! game's deferred directional-light pass; this module feeds it.
//!
//! Its per-frame values (sun, ambient, the water at the camera, caustics
//! frame) go into a small float texture every material binds; the texture's
//! contents are overwritten on the GPU each frame, so no material ever needs
//! to change. The 64 baked caustics frames are one array texture.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    Extent3d, TexelCopyBufferLayout, TextureDimension, TextureFormat, TextureViewDescriptor,
    TextureViewDimension,
};
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};
use sn_assets::TerrainTexture;

use crate::objects::GameDirectionalLight;
use crate::sky::{SkyState, light_rotation, linear, to_gamma};
use crate::sky_dome::SkyWorld;
use crate::textures::to_image;
use crate::water::{WaterFog, WaterWorld};
use crate::water_surface::WaterSurfaceWorld;

/// Directional lights of placed objects passed to the shaders at most.
const MAX_DIRECTIONAL: usize = 8;

/// Texels in the parameter texture; must match `game_light.wgsl`: 12 fixed,
/// a count, then direction and colour per directional light.
const PARAMS: usize = 13 + 2 * MAX_DIRECTIONAL;

/// The scene's sun `Light`: cookie size 10 m (read once with UnityPy on the
/// dev machine; `docs/formats/lighting.md`).
const COOKIE_SIZE: f32 = 10.0;

/// The textures every material binds for the game's lighting.
#[derive(Resource, Clone, ExtractResource)]
pub struct GameLightImages {
    pub params: Handle<Image>,
    pub caustics: Handle<Image>,
    /// Unity's default spot-light cookie (`Soft`), clamped.
    pub spot_cookie: Handle<Image>,
    /// Whether `caustics` holds the game's frames (else a placeholder).
    pub has_caustics: bool,
}

/// The lighting textures read from the game.
pub struct LightTextures {
    /// The caustics frames.
    pub caustics: Vec<TerrainTexture>,
    /// Unity's default spot-light cookie.
    pub spot_cookie: TerrainTexture,
}

/// The lighting textures read at start-up (`None`: none).
#[derive(Resource)]
pub struct PendingLightTextures(pub Option<LightTextures>);

/// This frame's parameter texels.
#[derive(Resource, Clone, Default, ExtractResource)]
struct GameLightParams([[f32; 4]; PARAMS]);

/// The caustics array: block-compressed frames with all their mips, one
/// layer each (the game's frames are DXT1, linear).
fn caustics_image(frames: &[TerrainTexture]) -> Option<Image> {
    let first = &frames.first()?.texture;
    let (w, h) = (first.width as u32, first.height as u32);
    let format = match first.format {
        10 => TextureFormat::Bc1RgbaUnorm,
        12 => TextureFormat::Bc3RgbaUnorm,
        _ => return None,
    };
    let block = if first.format == 10 { 8 } else { 16 };
    let mips = first.mip_count.max(1) as u32;
    let level_bytes = |level: u32| {
        let (mw, mh) = ((w >> level).max(1), (h >> level).max(1));
        mw.div_ceil(4) as usize * mh.div_ceil(4) as usize * block
    };
    let layer: usize = (0..mips).map(level_bytes).sum();
    let mut data = Vec::with_capacity(layer * frames.len());
    for f in frames {
        let t = &f.texture;
        if (t.width as u32, t.height as u32, t.format) != (w, h, first.format)
            || f.data.len() < layer
        {
            return None;
        }
        data.extend_from_slice(&f.data[..layer]);
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: frames.len() as u32,
        },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = mips;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.sampler = crate::textures::repeat_sampler();
    Some(image)
}

fn create_images(
    mut commands: Commands,
    mut pending: Option<ResMut<PendingLightTextures>>,
    mut images: ResMut<Assets<Image>>,
) {
    let textures = pending.as_mut().and_then(|p| p.0.take());
    let real = textures.as_ref().and_then(|t| caustics_image(&t.caustics));
    // The cookie is sampled clamped (its wrap mode), linear, sharpest mip.
    let spot_cookie = textures
        .as_ref()
        .and_then(|t| to_image(&t.spot_cookie, false))
        .map(|mut image| {
            image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
            image
        })
        .unwrap_or_else(|| {
            // No cookie: a white texel (spots then light their whole cone
            // area evenly; only without the game's resources).
            Image::new_fill(
                Extent3d::default(),
                TextureDimension::D2,
                &[255; 4],
                TextureFormat::Rgba8Unorm,
                RenderAssetUsages::RENDER_WORLD,
            )
        });
    let has_caustics = real.is_some();
    let caustics = real.unwrap_or_else(|| {
        // No caustics: one white layer (the shader then adds nothing).
        let mut image = Image::new_fill(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[0, 0, 0, 255],
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.texture_view_descriptor = Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..default()
        });
        image
    });
    let params = Image::new_fill(
        Extent3d {
            width: PARAMS as u32,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0; 16],
        TextureFormat::Rgba32Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    commands.insert_resource(GameLightImages {
        params: images.add(params),
        caustics: images.add(caustics),
        spot_cookie: images.add(spot_cookie),
        has_caustics,
    });
}

/// The game's global lighting values for this frame (`uSkyLight`,
/// `WaterscapeVolume.PreRender`, `WaterSurface`), at the camera.
#[allow(clippy::too_many_arguments)] // one parameter per source of values
fn update_params(
    time: Res<Time>,
    sky: Res<SkyState>,
    water: Res<WaterWorld>,
    sky_world: Option<Res<SkyWorld>>,
    surface: Option<Res<WaterSurfaceWorld>>,
    images: Option<Res<GameLightImages>>,
    cameras: Query<&WaterFog, With<Camera3d>>,
    directional: Query<(&GameDirectionalLight, &Transform)>,
    mut params: ResMut<GameLightParams>,
) {
    let Ok(fog) = cameras.single() else {
        return;
    };
    let unit = water.light_unit;
    let v = &water.volume;
    // The directional light's own direction (not the hour-angle sun).
    let light_dir = -sky.to_sun;
    // Caustics: one tile per `caustics_size` metres of the projection,
    // frames at the game's rate; baked frames are stored ÷ 6.
    let (frames, fps, tile) = surface.as_deref().map_or((1, 25, 2.5), |s| {
        (
            s.surface.num_caustics_frames.max(1),
            s.surface.caustics_frames_per_second.max(1),
            s.surface.caustics_size,
        )
    });
    let has_caustics = images.as_deref().is_some_and(|i| i.has_caustics);
    let frame =
        ((time.elapsed_secs_f64() * f64::from(fps)).floor() as i64).rem_euclid(i64::from(frames));
    // Without caustics the cookie stays 1 (amount 0).
    let (amount, texture_scale) = if has_caustics {
        (v.caustics_amount, 6.0)
    } else {
        (0.0, 0.0)
    };
    // World → light (Unity's directional cookie: the light's local x, y over
    // the cookie size, + 0.5), as rows over Bevy world coordinates.
    let rotation = sky_world
        .as_deref()
        .map_or(Quat::IDENTITY, |s| light_rotation(&s.manager, sky.timeline));
    let row = |axis: Vec3| {
        let a = rotation * axis / COOKIE_SIZE;
        [a.x, a.y, -a.z, 0.5]
    };
    params.0 = [
        light_dir.extend(0.0).to_array(),
        (sky.sun_light * unit).extend(unit).to_array(),
        (sky.top_ambient * unit).extend(fog.emissive.w).to_array(),
        (sky.bottom_ambient * unit)
            .extend(fog.scattering.w)
            .to_array(),
        fog.extinction.truncate().extend(fog.sun.w).to_array(),
        fog.emissive
            .truncate()
            .extend(v.emission_ambient_scale)
            .to_array(),
        [
            v.color_cast_distance_factor,
            v.color_cast_depth_factor,
            fog.misc.z,
            fog.misc.y,
        ],
        [
            v.caustics_scale / tile,
            amount,
            texture_scale * amount,
            frame as f32,
        ],
        row(Vec3::X),
        row(Vec3::Y),
        (sky.unity_ambient * unit).extend(0.0).to_array(),
        [sky.local_light, 0.0, 0.0, 0.0],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
    ];
    // Directional lights of loaded objects (the atmosphere volumes' bounce
    // lights): each lights the whole scene, as in the game.
    let mut count = 0;
    for (light, transform) in &directional {
        if count == MAX_DIRECTIONAL {
            break;
        }
        let colour = directional_colour(&light.0, sky.day_scalar) * unit;
        if colour.max_element() <= 0.0 {
            continue;
        }
        params.0[13 + 2 * count] = transform.forward().extend(0.0).to_array();
        params.0[14 + 2 * count] = colour.extend(0.0).to_array();
        count += 1;
    }
    params.0[12] = [count as f32, directional.iter().len() as f32, 0.0, 0.0];
}

/// Writes this frame's values into the parameter texture.
fn upload_params(
    params: Option<Res<GameLightParams>>,
    images: Option<Res<GameLightImages>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    queue: Res<RenderQueue>,
) {
    let (Some(params), Some(images)) = (params, images) else {
        return;
    };
    let Some(gpu) = gpu_images.get(&images.params) else {
        return;
    };
    let bytes: Vec<u8> = params
        .0
        .iter()
        .flatten()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    queue.write_texture(
        gpu.texture.as_image_copy(),
        &bytes,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some((PARAMS * 16) as u32),
            rows_per_image: Some(1),
        },
        Extent3d {
            width: PARAMS as u32,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
}

pub struct GameLightPlugin;

impl Plugin for GameLightPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "game_light.wgsl");
        app.add_plugins((
            ExtractResourcePlugin::<GameLightImages>::default(),
            ExtractResourcePlugin::<GameLightParams>::default(),
        ))
        .init_resource::<GameLightParams>()
        // Before any material is made (they all bind these images).
        .add_systems(PreStartup, create_images)
        .add_systems(
            Update,
            update_params
                .after(crate::water::update_water_fog)
                .run_if(resource_exists::<SkyState>)
                .run_if(resource_exists::<WaterWorld>),
        );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.add_systems(
            Render,
            upload_params.in_set(RenderSystems::PrepareResources),
        );
    }
}

/// A placed directional light's `_LightColor` at the day scalar `d`: its
/// `DayNightLight` (colour = lerp(curves, replace colour, sun fraction ×
/// replace fraction), intensity = `IntensityToGamma(I(d) × fade)` =
/// `LinearToGammaSpace(2 I(d) fade)`) or its stored values, then
/// `linear(colour × intensity)` (`m_LightsUseLinearIntensity` off).
fn directional_colour(light: &crate::objects::DirectionalSource, d: f32) -> Vec3 {
    let (colour, intensity) = match &light.day_night {
        Some(n) => {
            let curves = Vec3::new(
                n.color_r.evaluate(d),
                n.color_g.evaluate(d),
                n.color_b.evaluate(d),
            );
            let replace = Vec3::new(n.replace_color[0], n.replace_color[1], n.replace_color[2]);
            let t = (n.sun_fraction.evaluate(d) * n.replace_fraction).clamp(0.0, 1.0);
            (
                curves.lerp(replace, t),
                to_gamma(2.0 * n.intensity.evaluate(d) * n.fade),
            )
        }
        None => (Vec3::from(light.color), light.intensity),
    };
    linear(colour * intensity)
}

#[cfg(test)]
mod tests {
    /// The falloff of `unity_light_falloff` in game_light.wgsl, mirrored.
    fn falloff(distance: f32, range: f32) -> f32 {
        let t = distance * distance / (range * range);
        if t >= 1.0 {
            return 0.0;
        }
        let mut a = 1.0 / (1.0 + 25.0 * t);
        if t > 0.64 {
            a *= 1.0 - (t - 0.64) / 0.36;
        }
        a
    }

    #[test]
    fn light_falloff_at_known_distances() {
        // A light of range 10 m.
        let cases = [
            (0.0, 1.0),
            (2.0, 0.5),                                 // t = 0.04: 1 / (1 + 1)
            (5.0, 1.0 / 7.25),                          // t = 0.25
            (8.0, 1.0 / 17.0),                          // t = 0.64: start of the fade
            (9.0, (1.0 / 21.25) * (1.0 - 0.17 / 0.36)), // t = 0.81
            (10.0, 0.0),                                // at the range
            (12.0, 0.0),                                // beyond it
        ];
        for (d, want) in cases {
            let got = falloff(d, 10.0);
            assert!((got - want).abs() < 1e-6, "d = {d}: {got} != {want}");
        }
        // Continuous at the start of the fade, and falling everywhere.
        assert!((falloff(8.0 - 1e-4, 10.0) - falloff(8.0 + 1e-4, 10.0)).abs() < 1e-4);
        let mut last = 2.0;
        for i in 0..=100 {
            let a = falloff(i as f32 * 0.1, 10.0);
            assert!(a <= last, "falloff rises at {i}");
            last = a;
        }
    }

    #[test]
    fn shader_uses_the_same_falloff() {
        let src = include_str!("game_light.wgsl");
        for needle in [
            "if t >= 1.0",
            "1.0 / (1.0 + 25.0 * t)",
            "if t > 0.64",
            "1.0 - (t - 0.64) / 0.36",
            "dot(to_light, to_light) * (*light).color_inverse_square_range.w",
        ] {
            assert!(src.contains(needle), "game_light.wgsl lacks `{needle}`");
        }
    }
}
