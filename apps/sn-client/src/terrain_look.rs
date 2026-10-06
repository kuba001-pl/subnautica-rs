//! Real terrain materials: the game's textures (uploaded to the GPU in their
//! compressed form) and a triplanar shader (`terrain.wgsl`).

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use sn_assets::{TerrainMaterials, TerrainTexture};

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TriplanarExtension>;

/// Must match `TerrainParams` in `terrain.wgsl`.
#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
pub struct TriplanarParams {
    pub cap_tint: Vec4,
    pub side_tint: Vec4,
    pub cap_scale: f32,
    pub side_scale: f32,
    pub has_cap_normal: u32,
    pub has_side_normal: u32,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct TriplanarExtension {
    #[uniform(100)]
    pub params: TriplanarParams,
    #[texture(101)]
    #[sampler(102)]
    pub cap_albedo: Handle<Image>,
    #[texture(103)]
    #[sampler(104)]
    pub cap_normal: Handle<Image>,
    #[texture(105)]
    #[sampler(106)]
    pub side_albedo: Handle<Image>,
    #[texture(107)]
    #[sampler(108)]
    pub side_normal: Handle<Image>,
}

impl MaterialExtension for TriplanarExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://sn_client/terrain.wgsl".into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        "embedded://sn_client/terrain.wgsl".into()
    }
}

pub struct TerrainLookPlugin;

impl Plugin for TerrainLookPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "terrain.wgsl");
        app.add_plugins(MaterialPlugin::<TerrainMaterial>::default())
            .add_systems(Startup, build_look);
    }
}

/// The game's terrain materials, waiting to be turned into GPU resources.
#[derive(Resource)]
pub struct PendingTerrainLook(pub Option<TerrainMaterials>);

/// Material per octree type id (`None`: use the debug colour).
#[derive(Resource, Default)]
pub struct TerrainLook {
    pub by_type: Vec<Option<Handle<TerrainMaterial>>>,
    pub textures: usize,
    pub texture_bytes: usize,
}

fn sampler() -> ImageSampler {
    let mut descriptor = ImageSamplerDescriptor::linear();
    descriptor.set_address_mode(ImageAddressMode::Repeat);
    descriptor.mipmap_filter = ImageFilterMode::Linear;
    descriptor.set_anisotropic_filter(8);
    ImageSampler::Descriptor(descriptor)
}

/// A GPU image from a game texture: block-compressed data is uploaded as is,
/// with every complete mip level; other formats are decoded to RGBA (base
/// level only).
fn to_image(t: &TerrainTexture, srgb: bool) -> Option<Image> {
    let tex = &t.texture;
    let (w, h) = (tex.width as u32, tex.height as u32);
    let compressed = match (tex.format, srgb) {
        (10, true) => Some((TextureFormat::Bc1RgbaUnormSrgb, 8)),
        (10, false) => Some((TextureFormat::Bc1RgbaUnorm, 8)),
        (12, true) => Some((TextureFormat::Bc3RgbaUnormSrgb, 16)),
        (12, false) => Some((TextureFormat::Bc3RgbaUnorm, 16)),
        (25, true) => Some((TextureFormat::Bc7RgbaUnormSrgb, 16)),
        (25, false) => Some((TextureFormat::Bc7RgbaUnorm, 16)),
        _ => None,
    };
    let size = Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    let mut image = match compressed {
        // BC textures need their base size to be a multiple of the 4×4 block.
        Some((format, block_bytes)) if w % 4 == 0 && h % 4 == 0 => {
            let (mut mw, mut mh, mut total, mut levels) = (w, h, 0usize, 0u32);
            while levels < tex.mip_count.max(1) as u32 {
                let level = mw.div_ceil(4) as usize * mh.div_ceil(4) as usize * block_bytes;
                if total + level > t.data.len() {
                    break;
                }
                total += level;
                levels += 1;
                if mw == 1 && mh == 1 {
                    break;
                }
                mw = (mw / 2).max(1);
                mh = (mh / 2).max(1);
            }
            if levels == 0 {
                return None;
            }
            let mut image = Image::new_uninit(
                size,
                TextureDimension::D2,
                format,
                RenderAssetUsages::RENDER_WORLD,
            );
            image.data = Some(t.data[..total].to_vec());
            image.texture_descriptor.mip_level_count = levels;
            image
        }
        _ => {
            let rgba = tex.decode_rgba(&t.data).ok()?;
            let format = if srgb {
                TextureFormat::Rgba8UnormSrgb
            } else {
                TextureFormat::Rgba8Unorm
            };
            Image::new(
                size,
                TextureDimension::D2,
                rgba,
                format,
                RenderAssetUsages::RENDER_WORLD,
            )
        }
    };
    image.sampler = sampler();
    Some(image)
}

fn build_look(
    mut commands: Commands,
    mut pending: ResMut<PendingTerrainLook>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    let Some(source) = pending.0.take() else {
        commands.insert_resource(TerrainLook::default());
        return;
    };
    let white = images.add(Image::new(
        Extent3d::default(),
        TextureDimension::D2,
        vec![255; 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));
    let mut uploaded: Vec<(*const TerrainTexture, bool, Handle<Image>)> = Vec::new();
    let mut look = TerrainLook::default();
    let mut upload = |t: &Arc<TerrainTexture>, srgb: bool, look: &mut TerrainLook| {
        let key = Arc::as_ptr(t);
        if let Some((_, _, handle)) = uploaded.iter().find(|(k, s, _)| *k == key && *s == srgb) {
            return Some(handle.clone());
        }
        let image = to_image(t, srgb)?;
        look.textures += 1;
        look.texture_bytes += image.data.as_ref().map_or(0, Vec::len);
        let handle = images.add(image);
        uploaded.push((key, srgb, handle.clone()));
        Some(handle)
    };
    for slot in &source.types {
        let Some(m) = slot else {
            look.by_type.push(None);
            continue;
        };
        let cap_albedo = m
            .cap
            .albedo
            .as_ref()
            .and_then(|t| upload(t, true, &mut look));
        let side_albedo = m
            .side
            .albedo
            .as_ref()
            .and_then(|t| upload(t, true, &mut look));
        let cap_normal = m
            .cap
            .normal
            .as_ref()
            .and_then(|t| upload(t, false, &mut look));
        let side_normal = m
            .side
            .normal
            .as_ref()
            .and_then(|t| upload(t, false, &mut look));
        let (Some(cap_albedo), Some(side_albedo)) = (cap_albedo, side_albedo) else {
            look.by_type.push(None);
            continue;
        };
        let tint = |c: [f32; 4]| Vec4::new(c[0], c[1], c[2], 1.0);
        let handle = materials.add(ExtendedMaterial {
            base: StandardMaterial {
                perceptual_roughness: 0.85,
                ..default()
            },
            extension: TriplanarExtension {
                params: TriplanarParams {
                    cap_tint: tint(m.cap.tint),
                    side_tint: tint(m.side.tint),
                    cap_scale: m.cap.scale,
                    side_scale: m.side.scale,
                    has_cap_normal: u32::from(cap_normal.is_some()),
                    has_side_normal: u32::from(side_normal.is_some()),
                },
                cap_albedo,
                side_albedo,
                cap_normal: cap_normal.unwrap_or_else(|| white.clone()),
                side_normal: side_normal.unwrap_or_else(|| white.clone()),
            },
        });
        look.by_type.push(Some(handle));
    }
    info!(
        "terrain look: {} materials, {} textures ({:.0} MiB on the GPU)",
        look.by_type.iter().flatten().count(),
        look.textures,
        look.texture_bytes as f64 / 1048576.0
    );
    commands.insert_resource(look);
}
