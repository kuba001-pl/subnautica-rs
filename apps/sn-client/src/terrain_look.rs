//! Real terrain materials: the game's textures (uploaded to the GPU in their
//! compressed form) and a triplanar shader (`terrain.wgsl`).

use std::collections::HashMap;
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

/// The sun's illuminance (lux). Emission 1.0 in the game is as bright as a
/// white surface lit by it.
pub const SUN_ILLUMINANCE: f32 = 8_000.0;

/// Bits of `TriplanarParams::flags`; must match `terrain.wgsl`.
const CAP_SIDE: u32 = 1;
const CAP_NORMAL: u32 = 2;
const SIDE_NORMAL: u32 = 4;
const CAP_SIG: u32 = 8;
const SIDE_SIG: u32 = 16;

/// Must match `TerrainParams` in `terrain.wgsl`. Colours are linear.
#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
pub struct TriplanarParams {
    pub cap_tint: Vec4,
    pub side_tint: Vec4,
    pub border_tint: Vec4,
    pub cap_scale: f32,
    pub side_scale: f32,
    pub triplanar: f32,
    pub border_range: f32,
    pub border_offset: f32,
    pub inner_range: f32,
    pub inner_offset: f32,
    pub cap_range: f32,
    pub cap_offset: f32,
    pub cap_angle: f32,
    pub cap_emission: f32,
    pub side_emission: f32,
    pub emission_unit: f32,
    pub flags: u32,
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
    pub cap_sig: Handle<Image>,
    #[texture(107)]
    #[sampler(108)]
    pub side_albedo: Handle<Image>,
    #[texture(109)]
    #[sampler(110)]
    pub side_normal: Handle<Image>,
    #[texture(111)]
    #[sampler(112)]
    pub side_sig: Handle<Image>,
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

/// The game's terrain look: per octree type id, the material settings
/// (`None`: use the debug colour), and the GPU materials made from them.
#[derive(Resource, Default)]
pub struct TerrainLook {
    by_type: Vec<Option<TriplanarExtension>>,
    /// Per (type id, rank in the chunk's draw order).
    materials: HashMap<(u8, u8), Handle<TerrainMaterial>>,
    pub textures: usize,
    pub texture_bytes: usize,
}

impl TerrainLook {
    /// The material for `type_id` drawn `rank`-th in its chunk: rank 0 is
    /// opaque, later ranks are alpha-blended over it, in rank order.
    pub fn material(
        &mut self,
        type_id: u8,
        rank: u8,
        assets: &mut Assets<TerrainMaterial>,
    ) -> Option<Handle<TerrainMaterial>> {
        let extension = self.by_type.get(usize::from(type_id))?.as_ref()?;
        let handle = self.materials.entry((type_id, rank)).or_insert_with(|| {
            assets.add(ExtendedMaterial {
                base: StandardMaterial {
                    alpha_mode: if rank == 0 {
                        AlphaMode::Opaque
                    } else {
                        AlphaMode::Blend
                    },
                    // Transparent meshes are drawn by distance plus this
                    // bias. All layers of a batch share one bounding box
                    // (see terrain.rs), so this keeps the game's order. It
                    // also nudges each layer towards the camera, so coplanar
                    // layers pass the depth test.
                    depth_bias: f32::from(rank),
                    ..default()
                },
                extension: extension.clone(),
            })
        });
        Some(handle.clone())
    }
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

/// Unity stores material colours in sRGB and, in a linear-colour-space
/// project like Subnautica, converts them to linear for the shader.
fn linear(c: [f32; 4]) -> Vec4 {
    let l = Color::srgba(c[0], c[1], c[2], c[3]).to_linear();
    Vec4::new(l.red, l.green, l.blue, l.alpha)
}

fn build_look(
    mut commands: Commands,
    mut pending: ResMut<PendingTerrainLook>,
    mut images: ResMut<Assets<Image>>,
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
    let mut upload = |t: &Option<Arc<TerrainTexture>>, srgb: bool, look: &mut TerrainLook| {
        let t = t.as_ref()?;
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
        let cap_albedo = upload(&m.cap.albedo, true, &mut look);
        let side_albedo = upload(&m.side.albedo, true, &mut look);
        let cap_normal = upload(&m.cap.normal, false, &mut look);
        let side_normal = upload(&m.side.normal, false, &mut look);
        // Specular/illumination maps hold data, not colours.
        let cap_sig = upload(&m.cap.sig, false, &mut look);
        let side_sig = upload(&m.side.sig, false, &mut look);
        let (Some(cap_albedo), Some(side_albedo)) = (cap_albedo, side_albedo) else {
            look.by_type.push(None);
            continue;
        };
        let flag = |on: bool, bit: u32| if on { bit } else { 0 };
        let b = &m.blend;
        look.by_type.push(Some(TriplanarExtension {
            params: TriplanarParams {
                cap_tint: linear(m.cap.tint),
                side_tint: linear(m.side.tint),
                border_tint: linear(b.border_tint),
                cap_scale: m.cap.scale,
                side_scale: m.side.scale,
                triplanar: b.triplanar,
                border_range: b.border_range,
                border_offset: b.border_offset,
                inner_range: b.inner_range,
                inner_offset: b.inner_offset,
                cap_range: b.cap_range,
                cap_offset: b.cap_offset,
                cap_angle: b.cap_angle,
                cap_emission: m.cap.emission,
                side_emission: m.side.emission,
                emission_unit: SUN_ILLUMINANCE / std::f32::consts::PI,
                flags: flag(m.cap_side, CAP_SIDE)
                    | flag(cap_normal.is_some(), CAP_NORMAL)
                    | flag(side_normal.is_some(), SIDE_NORMAL)
                    | flag(cap_sig.is_some(), CAP_SIG)
                    | flag(side_sig.is_some(), SIDE_SIG),
            },
            cap_albedo,
            side_albedo,
            cap_normal: cap_normal.unwrap_or_else(|| white.clone()),
            side_normal: side_normal.unwrap_or_else(|| white.clone()),
            cap_sig: cap_sig.unwrap_or_else(|| white.clone()),
            side_sig: side_sig.unwrap_or_else(|| white.clone()),
        }));
    }
    info!(
        "terrain look: {} materials, {} textures ({:.0} MiB on the GPU)",
        look.by_type.iter().flatten().count(),
        look.textures,
        look.texture_bytes as f64 / 1048576.0
    );
    commands.insert_resource(look);
}
