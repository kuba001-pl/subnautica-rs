//! Game textures on the GPU: block-compressed data is uploaded as is, with
//! every complete mip level; other formats are decoded to RGBA.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sn_assets::TerrainTexture;

/// Unity stores material colours in sRGB and, in a linear-colour-space
/// project like Subnautica, converts them to linear for the shader.
pub fn linear(c: [f32; 4]) -> Vec4 {
    let l = Color::srgba(c[0], c[1], c[2], c[3]).to_linear();
    Vec4::new(l.red, l.green, l.blue, l.alpha)
}

/// Linear, repeating, trilinear, anisotropic ×8.
pub fn repeat_sampler() -> ImageSampler {
    let mut descriptor = ImageSamplerDescriptor::linear();
    descriptor.set_address_mode(ImageAddressMode::Repeat);
    descriptor.mipmap_filter = ImageFilterMode::Linear;
    descriptor.set_anisotropic_filter(8);
    ImageSampler::Descriptor(descriptor)
}

/// A GPU image from a game texture: block-compressed data is uploaded as is,
/// with every complete mip level; other formats are decoded to RGBA (base
/// level only).
pub fn to_image(t: &TerrainTexture, srgb: bool) -> Option<Image> {
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
    image.sampler = repeat_sampler();
    Some(image)
}
