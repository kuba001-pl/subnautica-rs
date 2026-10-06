//! `Texture2D` objects (class 28), Unity 2019.4 layout. The files carry no
//! type trees, so the field order is written out here; it is checked
//! against UnityPy (see `docs/formats/unity.md`).

use crate::reader::Reader;
use crate::{ErrorKind, Result};

/// Where pixel data lives when it is not inside the object: a slice of a
/// resource file (`*.resS`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamingInfo {
    pub offset: u64,
    pub size: u32,
    /// e.g. `archive:/CAB-…/CAB-….resS` (in a bundle) or `resources.assets.resS`.
    pub path: String,
}

impl StreamingInfo {
    /// The resource file's name without any `archive:/…/` prefix.
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture2D {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub complete_image_size: i32,
    /// Unity `TextureFormat` value; see [`TextureFormat`].
    pub format: i32,
    pub mip_count: i32,
    pub is_readable: bool,
    pub image_count: i32,
    pub dimension: i32,
    pub filter_mode: i32,
    pub aniso: i32,
    pub mip_bias: u32,
    /// Wrap modes U, V, W (0 repeat, 1 clamp, 2 mirror, 3 mirror once).
    pub wrap: [i32; 3],
    pub lightmap_format: i32,
    /// 0 gamma (sRGB), 1 linear.
    pub color_space: i32,
    /// Pixel data stored inside the object (empty if streamed).
    pub image_data: Vec<u8>,
    /// Pixel data stored in a resource file instead.
    pub stream: Option<StreamingInfo>,
}

/// The texture formats seen in Subnautica, by Unity's numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFormat {
    Alpha8,
    Rgb24,
    Rgba32,
    Argb32,
    Rgb565,
    Dxt1,
    Dxt5,
    Rgba4444,
    Bgra32,
    RHalf,
    RgbaHalf,
    RFloat,
    RgbaFloat,
    Bc6h,
    Bc7,
    Bc4,
    Bc5,
    Dxt1Crunched,
    Dxt5Crunched,
    R8,
    Other(i32),
}

impl TextureFormat {
    pub fn from_unity(value: i32) -> Self {
        match value {
            1 => Self::Alpha8,
            3 => Self::Rgb24,
            4 => Self::Rgba32,
            5 => Self::Argb32,
            7 => Self::Rgb565,
            10 => Self::Dxt1,
            12 => Self::Dxt5,
            13 => Self::Rgba4444,
            14 => Self::Bgra32,
            15 => Self::RHalf,
            17 => Self::RgbaHalf,
            18 => Self::RFloat,
            20 => Self::RgbaFloat,
            24 => Self::Bc6h,
            25 => Self::Bc7,
            26 => Self::Bc4,
            27 => Self::Bc5,
            28 => Self::Dxt1Crunched,
            29 => Self::Dxt5Crunched,
            63 => Self::R8,
            other => Self::Other(other),
        }
    }
}

impl Texture2D {
    pub fn texture_format(&self) -> TextureFormat {
        TextureFormat::from_unity(self.format)
    }

    /// Parses the object bytes of a Texture2D (from `SerializedFile::object_data`).
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Texture2D> {
        let mut r = Reader::new(data, big_endian);
        let name = r.aligned_string()?;
        let _forced_fallback_format = r.i32()?;
        let _downscale_fallback = r.u8()?;
        r.align(4)?;
        let width = r.i32()?;
        let height = r.i32()?;
        let complete_image_size = r.i32()?;
        let format = r.i32()?;
        let mip_count = r.i32()?;
        let is_readable = r.u8()? != 0;
        let _ignore_master_texture_limit = r.u8()?;
        let _streaming_mipmaps = r.u8()?;
        r.align(4)?;
        let _streaming_mipmaps_priority = r.i32()?;
        let image_count = r.i32()?;
        let dimension = r.i32()?;
        let filter_mode = r.i32()?;
        let aniso = r.i32()?;
        let mip_bias = r.u32()?;
        let wrap = [r.i32()?, r.i32()?, r.i32()?];
        let lightmap_format = r.i32()?;
        let color_space = r.i32()?;
        let size = r.count(1)?;
        let image_data = r.bytes(size)?.to_vec();
        r.align(4)?;
        let offset = u64::from(r.u32()?);
        let stream_size = r.u32()?;
        let path = r.aligned_string()?;
        if width < 0 || height < 0 {
            return Err(r.error(ErrorKind::Invalid(format!("texture size {width}x{height}"))));
        }
        let stream = (!path.is_empty()).then_some(StreamingInfo {
            offset,
            size: stream_size,
            path,
        });
        Ok(Texture2D {
            name,
            width,
            height,
            complete_image_size,
            format,
            mip_count,
            is_readable,
            image_count,
            dimension,
            filter_mode,
            aniso,
            mip_bias,
            wrap,
            lightmap_format,
            color_space,
            image_data,
            stream,
        })
    }
}

impl Texture2D {
    /// Size in bytes of the full-resolution image (mip level 0) in the stored
    /// format, or `None` for formats we don't handle.
    pub fn mip0_size(&self) -> Option<usize> {
        let (w, h) = (self.width as usize, self.height as usize);
        let blocks = w.div_ceil(4) * h.div_ceil(4);
        Some(match self.texture_format() {
            TextureFormat::Alpha8 | TextureFormat::R8 => w * h,
            TextureFormat::Rgb24 => w * h * 3,
            TextureFormat::Rgba32 | TextureFormat::Argb32 | TextureFormat::Bgra32 => w * h * 4,
            TextureFormat::Dxt1 | TextureFormat::Bc4 => blocks * 8,
            TextureFormat::Dxt5 | TextureFormat::Bc5 | TextureFormat::Bc7 => blocks * 16,
            _ => return None,
        })
    }

    /// Decodes mip level 0 to RGBA8, top row first (Unity stores the bottom
    /// row first; images are flipped to the usual orientation). `pixels` is
    /// the texture's pixel data: `image_data`, or the streamed slice.
    pub fn decode_rgba(&self, pixels: &[u8]) -> Result<Vec<u8>> {
        let (w, h) = (self.width as usize, self.height as usize);
        let needed = self.mip0_size().ok_or_else(|| crate::Error {
            offset: 0,
            kind: ErrorKind::Unsupported(format!("texture format {}", self.format)),
        })?;
        let data = pixels.get(..needed).ok_or(crate::Error {
            offset: pixels.len(),
            kind: ErrorKind::UnexpectedEof {
                needed: needed.saturating_sub(pixels.len()),
            },
        })?;
        let mut rgba = Vec::with_capacity(w * h * 4);
        match self.texture_format() {
            // Like a GPU sampling A8_UNORM (and UnityPy): colour 0, alpha from the data.
            TextureFormat::Alpha8 => data.iter().for_each(|&a| rgba.extend([0, 0, 0, a])),
            TextureFormat::R8 => data.iter().for_each(|&r| rgba.extend([r, 0, 0, 255])),
            TextureFormat::Rgb24 => data
                .chunks_exact(3)
                .for_each(|p| rgba.extend([p[0], p[1], p[2], 255])),
            TextureFormat::Rgba32 => rgba.extend_from_slice(data),
            TextureFormat::Argb32 => data
                .chunks_exact(4)
                .for_each(|p| rgba.extend([p[1], p[2], p[3], p[0]])),
            TextureFormat::Bgra32 => data
                .chunks_exact(4)
                .for_each(|p| rgba.extend([p[2], p[1], p[0], p[3]])),
            format => {
                let mut image = vec![0u32; w * h];
                let decoded = match format {
                    TextureFormat::Dxt1 => texture2ddecoder::decode_bc1(data, w, h, &mut image),
                    TextureFormat::Dxt5 => texture2ddecoder::decode_bc3(data, w, h, &mut image),
                    TextureFormat::Bc4 => texture2ddecoder::decode_bc4(data, w, h, &mut image),
                    TextureFormat::Bc5 => texture2ddecoder::decode_bc5(data, w, h, &mut image),
                    TextureFormat::Bc7 => texture2ddecoder::decode_bc7(data, w, h, &mut image),
                    _ => Err("unsupported format"),
                };
                decoded.map_err(|e| crate::Error {
                    offset: 0,
                    kind: ErrorKind::Decompress(e.into()),
                })?;
                // The decoder packs pixels as little-endian B, G, R, A.
                for p in image {
                    let [b, g, r, a] = p.to_le_bytes();
                    rgba.extend([r, g, b, a]);
                }
            }
        }
        // Flip rows: bottom-up storage → top-down image.
        let row = w * 4;
        let mut flipped = Vec::with_capacity(rgba.len());
        for y in (0..h).rev() {
            flipped.extend_from_slice(&rgba[y * row..(y + 1) * row]);
        }
        Ok(flipped)
    }
}
