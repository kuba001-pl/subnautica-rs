//! The game's UI objects (M9u, `docs/formats/ui.md`): `RectTransform`
//! (class 224), `Canvas` (223), `CanvasGroup` (225), `CanvasRenderer`
//! (222), `Sprite` (213), `SpriteAtlas` (687078895), and the game's
//! `uGUI_CanvasScaler` script. Unity 2019.4 layouts without type trees
//! (field order checked against UnityPy's type-tree database for
//! 2019.4.36f1, M9u2); each parser reads every field and requires the
//! data to end with the last one.

use crate::gameplay::{at_end, fields};
use crate::objects::PPtr;
use crate::reader::Reader;
use crate::{ErrorKind, Result};

fn vec2(r: &mut Reader) -> Result<[f32; 2]> {
    Ok([r.f32()?, r.f32()?])
}

fn vec4(r: &mut Reader) -> Result<[f32; 4]> {
    Ok([r.f32()?, r.f32()?, r.f32()?, r.f32()?])
}

fn pptrs(r: &mut Reader) -> Result<Vec<PPtr>> {
    let n = r.count(12)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(PPtr::read(r)?);
    }
    Ok(out)
}

fn strings(r: &mut Reader) -> Result<Vec<String>> {
    let n = r.count(4)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(r.aligned_string()?);
    }
    Ok(out)
}

/// A sprite's render data key: the atlas finds the sprite's packed place
/// by it (`m_RenderDataKey`, `SpriteAtlas.m_RenderDataMap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenderDataKey {
    pub guid: [u32; 4],
    pub id: i64,
}

impl RenderDataKey {
    fn read(r: &mut Reader) -> Result<RenderDataKey> {
        Ok(RenderDataKey {
            guid: [r.u32()?, r.u32()?, r.u32()?, r.u32()?],
            id: r.i64()?,
        })
    }
}

/// `RectTransform`: a Transform with its rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct RectTransform {
    pub game_object: PPtr,
    /// Quaternion x, y, z, w.
    pub rotation: [f32; 4],
    pub position: [f32; 3],
    pub scale: [f32; 3],
    pub children: Vec<PPtr>,
    pub father: PPtr,
    pub anchor_min: [f32; 2],
    pub anchor_max: [f32; 2],
    pub anchored_position: [f32; 2],
    pub size_delta: [f32; 2],
    pub pivot: [f32; 2],
}

impl RectTransform {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<RectTransform> {
        let mut r = Reader::new(data, big_endian);
        let game_object = PPtr::read(&mut r)?;
        let rotation = vec4(&mut r)?;
        let position = [r.f32()?, r.f32()?, r.f32()?];
        let scale = [r.f32()?, r.f32()?, r.f32()?];
        let children = pptrs(&mut r)?;
        let father = PPtr::read(&mut r)?;
        let t = RectTransform {
            game_object,
            rotation,
            position,
            scale,
            children,
            father,
            anchor_min: vec2(&mut r)?,
            anchor_max: vec2(&mut r)?,
            anchored_position: vec2(&mut r)?,
            size_delta: vec2(&mut r)?,
            pivot: vec2(&mut r)?,
        };
        at_end(&r, data)?;
        Ok(t)
    }
}

/// `CanvasGroup`.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasGroup {
    pub game_object: PPtr,
    pub enabled: bool,
    pub alpha: f32,
    pub interactable: bool,
    pub blocks_raycasts: bool,
    pub ignore_parent_groups: bool,
}

impl CanvasGroup {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<CanvasGroup> {
        let mut r = Reader::new(data, big_endian);
        let game_object = PPtr::read(&mut r)?;
        let enabled = r.bool4()?;
        let alpha = r.f32()?;
        let interactable = r.u8()? != 0;
        let blocks_raycasts = r.u8()? != 0;
        let ignore_parent_groups = r.bool4()?;
        at_end(&r, data)?;
        Ok(CanvasGroup {
            game_object,
            enabled,
            alpha,
            interactable,
            blocks_raycasts,
            ignore_parent_groups,
        })
    }
}

/// `CanvasRenderer`.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasRenderer {
    pub game_object: PPtr,
    pub cull_transparent_mesh: bool,
}

impl CanvasRenderer {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<CanvasRenderer> {
        let mut r = Reader::new(data, big_endian);
        let game_object = PPtr::read(&mut r)?;
        let cull_transparent_mesh = r.u8()? != 0;
        // Stored with or without padding after the flag.
        if r.pos() < data.len() {
            r.align(4)?;
        }
        at_end(&r, data)?;
        Ok(CanvasRenderer {
            game_object,
            cull_transparent_mesh,
        })
    }
}

/// Where a sprite's pixels are: its texture (an atlas page once packed),
/// the rectangle in it, and how it was packed.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteRenderData {
    pub texture: PPtr,
    pub alpha_texture: PPtr,
    /// Pixels: x, y (from the bottom), width, height.
    pub texture_rect: [f32; 4],
    pub texture_rect_offset: [f32; 2],
    pub atlas_rect_offset: [f32; 2],
    /// Bit 0 packed, bit 1 packing mode (0 tight, 1 rectangle), bits 2–5
    /// rotation (0 none, 1 flip horizontal, 2 flip vertical, 3 rotate 180,
    /// 4 rotate 90), bit 6 mesh type (0 full rect, 1 tight).
    pub settings_raw: u32,
    pub uv_transform: [f32; 4],
    pub downscale_multiplier: f32,
}

impl SpriteRenderData {
    pub fn packed(&self) -> bool {
        self.settings_raw & 1 != 0
    }

    /// 0 tight, 1 rectangle.
    pub fn packing_mode(&self) -> u32 {
        (self.settings_raw >> 1) & 1
    }

    pub fn packing_rotation(&self) -> u32 {
        (self.settings_raw >> 2) & 0xF
    }
}

/// A sprite's mesh (vertex positions and UVs, in its own vertex format).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpriteMesh {
    /// (first byte, index count) per submesh.
    pub submeshes: Vec<(u32, u32)>,
    /// 16-bit indices.
    pub index_buffer: Vec<u8>,
    pub vertex_count: u32,
    /// (stream, offset, format, dimension) per channel.
    pub channels: Vec<[u8; 4]>,
    pub vertex_data: Vec<u8>,
}

/// `Sprite`.
#[derive(Clone, Debug, PartialEq)]
pub struct Sprite {
    pub name: String,
    /// Pixels in its source texture: x, y (from the bottom), width,
    /// height.
    pub rect: [f32; 4],
    pub offset: [f32; 2],
    /// Nine-slice borders in pixels: left, bottom, right, top.
    pub border: [f32; 4],
    pub pixels_to_units: f32,
    /// 0..1 of the rect.
    pub pivot: [f32; 2],
    pub extrude: u32,
    pub is_polygon: bool,
    pub render_data_key: RenderDataKey,
    pub atlas_tags: Vec<String>,
    pub sprite_atlas: PPtr,
    /// The render data stored with the sprite (its source texture until
    /// an atlas packs it; then the atlas holds the packed one).
    pub rd: SpriteRenderData,
    pub mesh: SpriteMesh,
}

impl Sprite {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Sprite> {
        let mut r = Reader::new(data, big_endian);
        let name = r.aligned_string()?;
        let rect = vec4(&mut r)?;
        let offset = vec2(&mut r)?;
        let border = vec4(&mut r)?;
        let pixels_to_units = r.f32()?;
        let pivot = vec2(&mut r)?;
        let extrude = r.u32()?;
        let is_polygon = r.bool4()?;
        let render_data_key = RenderDataKey::read(&mut r)?;
        let atlas_tags = strings(&mut r)?;
        let sprite_atlas = PPtr::read(&mut r)?;
        // m_RD
        let texture = PPtr::read(&mut r)?;
        let alpha_texture = PPtr::read(&mut r)?;
        let n = r.count(16)?;
        for _ in 0..n {
            PPtr::read(&mut r)?; // secondary texture
            r.aligned_string()?; // its name
        }
        let n = r.count(48)?;
        let mut submeshes = Vec::with_capacity(n);
        for _ in 0..n {
            let first_byte = r.u32()?;
            let index_count = r.u32()?;
            r.bytes(16 + 24)?; // topology, base and first vertex, count, bounds
            submeshes.push((first_byte, index_count));
        }
        let n = r.count(1)?;
        let index_buffer = r.bytes(n)?.to_vec();
        r.align(4)?;
        let vertex_count = r.u32()?;
        let n = r.count(4)?;
        let mut channels = Vec::with_capacity(n);
        for _ in 0..n {
            channels.push(r.array::<4>()?);
        }
        let n = r.count(1)?;
        let vertex_data = r.bytes(n)?.to_vec();
        r.align(4)?;
        let n = r.count(64)?;
        r.bytes(n * 64)?; // bind poses
        let texture_rect = vec4(&mut r)?;
        let texture_rect_offset = vec2(&mut r)?;
        let atlas_rect_offset = vec2(&mut r)?;
        let settings_raw = r.u32()?;
        let uv_transform = vec4(&mut r)?;
        let downscale_multiplier = r.f32()?;
        // m_PhysicsShape: outlines of points.
        let n = r.count(4)?;
        for _ in 0..n {
            let points = r.count(8)?;
            r.bytes(points * 8)?;
        }
        // m_Bones: none in this game; read to stay exact.
        let n = r.count(4 + 12 + 16 + 4 + 4)?;
        for _ in 0..n {
            r.aligned_string()?;
            r.bytes(12 + 16 + 4 + 4)?;
        }
        at_end(&r, data)?;
        Ok(Sprite {
            name,
            rect,
            offset,
            border,
            pixels_to_units,
            pivot,
            extrude,
            is_polygon,
            render_data_key,
            atlas_tags,
            sprite_atlas,
            rd: SpriteRenderData {
                texture,
                alpha_texture,
                texture_rect,
                texture_rect_offset,
                atlas_rect_offset,
                settings_raw,
                uv_transform,
                downscale_multiplier,
            },
            mesh: SpriteMesh {
                submeshes,
                index_buffer,
                vertex_count,
                channels,
                vertex_data,
            },
        })
    }
}

/// `SpriteAtlas`: the sprites it packs and, per sprite, where it is
/// packed.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteAtlas {
    pub name: String,
    pub packed_sprites: Vec<PPtr>,
    pub packed_sprite_names: Vec<String>,
    pub render_data: Vec<(RenderDataKey, SpriteRenderData)>,
    pub tag: String,
    pub is_variant: bool,
}

impl SpriteAtlas {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<SpriteAtlas> {
        let mut r = Reader::new(data, big_endian);
        let name = r.aligned_string()?;
        let packed_sprites = pptrs(&mut r)?;
        let packed_sprite_names = strings(&mut r)?;
        if packed_sprite_names.len() != packed_sprites.len() {
            return Err(r.error(ErrorKind::Invalid(format!(
                "{} sprites but {} names",
                packed_sprites.len(),
                packed_sprite_names.len()
            ))));
        }
        let n = r.count(104)?;
        let mut render_data = Vec::with_capacity(n);
        for _ in 0..n {
            let key = RenderDataKey::read(&mut r)?;
            let texture = PPtr::read(&mut r)?;
            let alpha_texture = PPtr::read(&mut r)?;
            let texture_rect = vec4(&mut r)?;
            let texture_rect_offset = vec2(&mut r)?;
            let atlas_rect_offset = vec2(&mut r)?;
            let uv_transform = vec4(&mut r)?;
            let downscale_multiplier = r.f32()?;
            let settings_raw = r.u32()?;
            render_data.push((
                key,
                SpriteRenderData {
                    texture,
                    alpha_texture,
                    texture_rect,
                    texture_rect_offset,
                    atlas_rect_offset,
                    settings_raw,
                    uv_transform,
                    downscale_multiplier,
                },
            ));
        }
        let tag = r.aligned_string()?;
        let is_variant = r.bool4()?;
        at_end(&r, data)?;
        Ok(SpriteAtlas {
            name,
            packed_sprites,
            packed_sprite_names,
            render_data,
            tag,
            is_variant,
        })
    }

    /// Where `key`'s sprite is packed.
    pub fn render_data_of(&self, key: &RenderDataKey) -> Option<&SpriteRenderData> {
        self.render_data
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, d)| d)
    }
}

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
    fn reads_an_atlas() {
        let mut b = Vec::new();
        string(&mut b, "Items");
        b.extend_from_slice(&2u32.to_le_bytes());
        pptr(&mut b, 1, 10);
        pptr(&mut b, 1, 11);
        b.extend_from_slice(&2u32.to_le_bytes());
        string(&mut b, "titanium");
        string(&mut b, "copper");
        b.extend_from_slice(&1u32.to_le_bytes());
        for v in [1u32, 2, 3, 4] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&5i64.to_le_bytes());
        pptr(&mut b, 0, 7);
        pptr(&mut b, 0, 0);
        for v in [
            10f32, 20.0, 64.0, 64.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.5, 1.0, 0.5, 1.0,
        ] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&3u32.to_le_bytes());
        string(&mut b, "");
        b.extend_from_slice(&[0, 0, 0, 0]);
        let a = SpriteAtlas::parse(&b, false).unwrap();
        assert_eq!(a.name, "Items");
        assert_eq!(a.packed_sprite_names, ["titanium", "copper"]);
        let key = RenderDataKey {
            guid: [1, 2, 3, 4],
            id: 5,
        };
        let d = a.render_data_of(&key).unwrap();
        assert_eq!(d.texture.path_id, 7);
        assert_eq!(d.texture_rect, [10.0, 20.0, 64.0, 64.0]);
        assert!(d.packed());
        assert_eq!(d.packing_mode(), 1);
        assert_eq!(d.packing_rotation(), 0);
        assert_eq!(object_name(&b, false).unwrap(), "Items");
        for len in [0, 3, 12, 30, 40, 100, b.len() - 1] {
            assert!(SpriteAtlas::parse(&b[..len], false).is_err(), "{len}");
        }
        let mut huge = Vec::new();
        string(&mut huge, "x");
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(SpriteAtlas::parse(&huge, false).is_err());
    }

    #[test]
    fn reads_a_rect_transform_and_a_canvas_group() {
        let mut b = Vec::new();
        pptr(&mut b, 0, 4);
        for v in [0f32, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0, 1.0, 1.0, 1.0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&1u32.to_le_bytes());
        pptr(&mut b, 0, 9);
        pptr(&mut b, 0, 2);
        for v in [0f32, 0.0, 1.0, 1.0, 5.0, -5.0, 100.0, 50.0, 0.5, 0.5] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let t = RectTransform::parse(&b, false).unwrap();
        assert_eq!(t.children.len(), 1);
        assert_eq!(t.father.path_id, 2);
        assert_eq!(t.anchor_max, [1.0, 1.0]);
        assert_eq!(t.anchored_position, [5.0, -5.0]);
        assert_eq!(t.size_delta, [100.0, 50.0]);
        assert_eq!(t.pivot, [0.5, 0.5]);
        for len in [0, 40, b.len() - 1] {
            assert!(RectTransform::parse(&b[..len], false).is_err());
        }
        let mut g = Vec::new();
        pptr(&mut g, 0, 4);
        g.extend_from_slice(&[1, 0, 0, 0]);
        g.extend_from_slice(&0.5f32.to_le_bytes());
        g.extend_from_slice(&[1, 0, 1, 0]);
        let c = CanvasGroup::parse(&g, false).unwrap();
        assert_eq!(c.alpha, 0.5);
        assert!(c.interactable && !c.blocks_raycasts && c.ignore_parent_groups);
        assert!(CanvasGroup::parse(&g[..22], false).is_err());
        let mut cr = Vec::new();
        pptr(&mut cr, 0, 4);
        cr.push(1);
        assert!(
            CanvasRenderer::parse(&cr, false)
                .unwrap()
                .cull_transparent_mesh
        );
        cr.extend_from_slice(&[0, 0, 0]);
        assert!(CanvasRenderer::parse(&cr, false).is_ok());
        cr.extend_from_slice(&[0; 4]);
        assert!(CanvasRenderer::parse(&cr, false).is_err());
    }

    #[test]
    fn reads_a_sprite() {
        let mut b = Vec::new();
        string(&mut b, "titanium");
        for v in [
            0f32, 0.0, 128.0, 128.0, 0.0, 0.0, 4.0, 4.0, 4.0, 4.0, 100.0, 0.5, 0.5,
        ] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&1u32.to_le_bytes()); // extrude
        b.extend_from_slice(&[0, 0, 0, 0]); // polygon
        for v in [1u32, 2, 3, 4] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&5i64.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        string(&mut b, "Items");
        pptr(&mut b, 0, 3); // atlas
        pptr(&mut b, 0, 8); // texture
        pptr(&mut b, 0, 0);
        b.extend_from_slice(&0u32.to_le_bytes()); // secondary
        b.extend_from_slice(&1u32.to_le_bytes()); // submeshes
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&6u32.to_le_bytes());
        b.extend_from_slice(&[0; 40]);
        b.extend_from_slice(&12u32.to_le_bytes()); // index buffer
        b.extend_from_slice(&[0, 0, 1, 0, 2, 0, 2, 0, 3, 0, 0, 0]);
        b.extend_from_slice(&4u32.to_le_bytes()); // vertices
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&[0, 0, 0, 3]);
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&[9, 9, 0, 0]);
        b.extend_from_slice(&0u32.to_le_bytes()); // bind poses
        for v in [0f32, 0.0, 128.0, 128.0, 0.0, 0.0, 0.0, 0.0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&0x40u32.to_le_bytes());
        for v in [100f32, 64.0, 100.0, 64.0, 1.0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&1u32.to_le_bytes()); // physics shape
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&[0; 8]);
        b.extend_from_slice(&0u32.to_le_bytes()); // bones
        let s = Sprite::parse(&b, false).unwrap();
        assert_eq!(s.name, "titanium");
        assert_eq!(s.rect, [0.0, 0.0, 128.0, 128.0]);
        assert_eq!(s.border, [4.0; 4]);
        assert_eq!(s.pixels_to_units, 100.0);
        assert_eq!(s.render_data_key.id, 5);
        assert_eq!(s.atlas_tags, ["Items"]);
        assert_eq!(s.sprite_atlas.path_id, 3);
        assert_eq!(s.rd.texture.path_id, 8);
        assert_eq!(s.rd.settings_raw, 0x40);
        assert!(!s.rd.packed());
        assert_eq!(s.mesh.submeshes, [(0, 6)]);
        assert_eq!(s.mesh.index_buffer.len(), 12);
        assert_eq!(s.mesh.vertex_count, 4);
        assert_eq!(s.mesh.vertex_data, [9, 9]);
        for len in [0, 20, 100, b.len() - 1] {
            assert!(Sprite::parse(&b[..len], false).is_err(), "{len}");
        }
    }
}
