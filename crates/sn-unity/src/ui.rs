//! The game's UI objects (M9u, `docs/formats/ui.md`): `Canvas` (class
//! 223), the game's `uGUI_CanvasScaler` script, and the head of
//! `SpriteAtlas` (class 687078895). Unity 2019.4 layouts without type
//! trees; each parser that reads every field requires the data to end
//! with the last one.

use crate::gameplay::{at_end, fields};
use crate::objects::PPtr;
use crate::reader::Reader;
use crate::{ErrorKind, Result};

/// `RenderMode`.
pub const RENDER_MODE_SCREEN_SPACE_OVERLAY: i32 = 0;
pub const RENDER_MODE_SCREEN_SPACE_CAMERA: i32 = 1;
pub const RENDER_MODE_WORLD_SPACE: i32 = 2;

/// `Canvas`, every field.
#[derive(Clone, Debug, PartialEq)]
pub struct Canvas {
    pub game_object: PPtr,
    pub enabled: bool,
    /// [`RENDER_MODE_SCREEN_SPACE_OVERLAY`], `…_CAMERA` or `…_WORLD_SPACE`.
    pub render_mode: i32,
    pub camera: PPtr,
    pub plane_distance: f32,
    pub pixel_perfect: bool,
    pub receives_events: bool,
    pub override_sorting: bool,
    pub override_pixel_perfect: bool,
    pub sorting_bucket_normalized_size: f32,
    /// `AdditionalCanvasShaderChannels` bits (1 TexCoord1, 2 TexCoord2,
    /// 4 TexCoord3, 8 Normal, 16 Tangent).
    pub additional_shader_channels: u32,
    pub sorting_layer_id: i32,
    pub sorting_order: i16,
    pub target_display: i8,
}

impl Canvas {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Canvas> {
        let mut r = Reader::new(data, big_endian);
        let game_object = PPtr::read(&mut r)?;
        let enabled = r.bool4()?;
        let render_mode = r.i32()?;
        let camera = PPtr::read(&mut r)?;
        let plane_distance = r.f32()?;
        let pixel_perfect = r.u8()? != 0;
        let receives_events = r.u8()? != 0;
        let override_sorting = r.u8()? != 0;
        let override_pixel_perfect = r.u8()? != 0;
        let sorting_bucket_normalized_size = r.f32()?;
        let additional_shader_channels = r.u32()?;
        let sorting_layer_id = r.i32()?;
        let sorting_order = r.i16()?;
        let target_display = r.u8()? as i8;
        at_end(&r, data)?;
        Ok(Canvas {
            game_object,
            enabled,
            render_mode,
            camera,
            plane_distance,
            pixel_perfect,
            receives_events,
            override_sorting,
            override_pixel_perfect,
            sorting_bucket_normalized_size,
            additional_shader_channels,
            sorting_layer_id,
            sorting_order,
            target_display,
        })
    }
}

/// The game's `uGUI_CanvasScaler` (a MonoBehaviour): its serialized
/// fields, in declaration order (`sn-inspect code --fields`).
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasScaler {
    pub enabled: bool,
    pub reference_resolution: [f32; 2],
    /// `uGUI_CanvasScaler.Mode`.
    pub mode: i32,
    pub vr_mode: i32,
    /// Metres in front of the UI camera (`UpdateTransform`).
    pub distance: f32,
    /// `uGUI_CanvasScaler.ScaleMode`.
    pub scale_mode: i32,
}

impl CanvasScaler {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<CanvasScaler> {
        let header = crate::objects::MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = fields(data, big_endian)?;
        let reference_resolution = [r.f32()?, r.f32()?];
        let mode = r.i32()?;
        let vr_mode = r.i32()?;
        let distance = r.f32()?;
        let scale_mode = r.i32()?;
        at_end(&r, data)?;
        Ok(CanvasScaler {
            enabled: header.enabled,
            reference_resolution,
            mode,
            vr_mode,
            distance,
            scale_mode,
        })
    }
}

/// The first fields of a `SpriteAtlas`: its name, the sprites it packs and
/// their names. The render data map that follows is read in M9u2.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteAtlasHead {
    pub name: String,
    pub packed_sprites: Vec<PPtr>,
    pub packed_sprite_names: Vec<String>,
}

impl SpriteAtlasHead {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<SpriteAtlasHead> {
        let mut r = Reader::new(data, big_endian);
        let name = r.aligned_string()?;
        let n = r.count(12)?;
        let mut packed_sprites = Vec::with_capacity(n);
        for _ in 0..n {
            packed_sprites.push(PPtr::read(&mut r)?);
        }
        let n = r.count(4)?;
        let mut packed_sprite_names = Vec::with_capacity(n);
        for _ in 0..n {
            packed_sprite_names.push(r.aligned_string()?);
        }
        if packed_sprite_names.len() != packed_sprites.len() {
            return Err(r.error(ErrorKind::Invalid(format!(
                "{} sprites but {} names",
                packed_sprites.len(),
                packed_sprite_names.len()
            ))));
        }
        Ok(SpriteAtlasHead {
            name,
            packed_sprites,
            packed_sprite_names,
        })
    }
}

/// The name of a named object (`m_Name` is the first field of assets
/// such as `Font`, `Sprite`, `Texture2D`, `Material`).
pub fn object_name(data: &[u8], big_endian: bool) -> Result<String> {
    Reader::new(data, big_endian).aligned_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pptr(b: &mut Vec<u8>, file: i32, path: i64) {
        b.extend_from_slice(&file.to_le_bytes());
        b.extend_from_slice(&path.to_le_bytes());
    }

    fn string(b: &mut Vec<u8>, s: &str) {
        b.extend_from_slice(&(s.len() as u32).to_le_bytes());
        b.extend_from_slice(s.as_bytes());
        while b.len() % 4 != 0 {
            b.push(0);
        }
    }

    /// A canvas as Unity 2019.4 stores it (55 bytes).
    fn canvas_bytes(mode: i32, order: i16) -> Vec<u8> {
        let mut b = Vec::new();
        pptr(&mut b, 0, 7);
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&mode.to_le_bytes());
        pptr(&mut b, 0, 0);
        b.extend_from_slice(&100f32.to_le_bytes());
        b.extend_from_slice(&[0, 1, 1, 0]);
        b.extend_from_slice(&0f32.to_le_bytes());
        b.extend_from_slice(&25u32.to_le_bytes());
        b.extend_from_slice(&(-362_810_557i32).to_le_bytes());
        b.extend_from_slice(&order.to_le_bytes());
        b.push(0);
        b
    }

    #[test]
    fn reads_a_canvas() {
        let b = canvas_bytes(RENDER_MODE_WORLD_SPACE, 1000);
        assert_eq!(b.len(), 55);
        let c = Canvas::parse(&b, false).unwrap();
        assert_eq!(c.game_object.path_id, 7);
        assert!(c.enabled);
        assert_eq!(c.render_mode, RENDER_MODE_WORLD_SPACE);
        assert_eq!(c.plane_distance, 100.0);
        assert!(!c.pixel_perfect && c.receives_events && c.override_sorting);
        assert_eq!(c.additional_shader_channels, 25);
        assert_eq!(c.sorting_layer_id, -362_810_557);
        assert_eq!(c.sorting_order, 1000);
    }

    #[test]
    fn bad_canvases_are_errors() {
        let b = canvas_bytes(0, 0);
        for len in [0, 12, 30, 54] {
            assert!(Canvas::parse(&b[..len], false).is_err(), "length {len}");
        }
        let mut longer = b.clone();
        longer.push(0);
        assert!(Canvas::parse(&longer, false).is_err());
    }

    fn scaler_bytes() -> Vec<u8> {
        let mut b = Vec::new();
        pptr(&mut b, 0, 3);
        b.extend_from_slice(&[1, 0, 0, 0]);
        pptr(&mut b, 1, 99);
        string(&mut b, "");
        for v in [1920f32, 1080.0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&1.5f32.to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes());
        b
    }

    #[test]
    fn reads_a_canvas_scaler() {
        let b = scaler_bytes();
        let s = CanvasScaler::parse(&b, false).unwrap();
        assert!(s.enabled);
        assert_eq!(s.reference_resolution, [1920.0, 1080.0]);
        assert_eq!((s.mode, s.vr_mode, s.scale_mode), (1, 2, 0));
        assert_eq!(s.distance, 1.5);
        for len in [0, 20, b.len() - 1] {
            assert!(CanvasScaler::parse(&b[..len], false).is_err());
        }
    }

    #[test]
    fn reads_an_atlas_head() {
        let mut b = Vec::new();
        string(&mut b, "Items");
        b.extend_from_slice(&2u32.to_le_bytes());
        pptr(&mut b, 1, 10);
        pptr(&mut b, 1, 11);
        b.extend_from_slice(&2u32.to_le_bytes());
        string(&mut b, "titanium");
        string(&mut b, "copper");
        b.extend_from_slice(&[0; 8]); // the render data map and the rest
        let a = SpriteAtlasHead::parse(&b, false).unwrap();
        assert_eq!(a.name, "Items");
        assert_eq!(a.packed_sprites.len(), 2);
        assert_eq!(a.packed_sprite_names, ["titanium", "copper"]);
        assert_eq!(object_name(&b, false).unwrap(), "Items");
        for len in [0, 3, 12, 30, 40] {
            assert!(SpriteAtlasHead::parse(&b[..len], false).is_err(), "{len}");
        }
        // A count larger than the data is an error, not an allocation.
        let mut huge = Vec::new();
        string(&mut huge, "x");
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(SpriteAtlasHead::parse(&huge, false).is_err());
    }
}
