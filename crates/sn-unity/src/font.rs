//! `Font` (class 128, M9u3, `docs/formats/ui.md` § Fonts): Unity's font
//! object, which holds the font file's bytes. TextMesh Pro's font assets
//! point at it (`m_SourceFontFile`) and make their glyphs from those
//! bytes. Unity 2019.4 layout without type trees (field order checked
//! against UnityPy's type-tree database for 2019.4.36f1); the data must
//! end with the last field.

use crate::Result;
use crate::gameplay::at_end;
use crate::objects::PPtr;
use crate::reader::Reader;

/// `Font`. The legacy (non-TMP) fields are read and kept only where
/// cheap; the game's text is all TMP.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub name: String,
    pub line_spacing: f32,
    pub default_material: PPtr,
    pub font_size: f32,
    pub texture: PPtr,
    /// `m_CharacterRects`: the legacy dynamic font's glyph count.
    pub character_rects: usize,
    /// `m_KerningValues`: pair count.
    pub kerning_pairs: usize,
    pub pixel_scale: f32,
    /// `m_FontData`: the TrueType / OpenType file.
    pub data: Vec<u8>,
    pub ascent: f32,
    pub descent: f32,
    pub default_style: u32,
    pub font_names: Vec<String>,
    pub fallback_fonts: Vec<PPtr>,
    pub rendering_mode: i32,
    pub use_legacy_bounds: bool,
    pub round_advance: bool,
}

/// `CharacterInfo`: index, uv rect, vert rect, advance, flipped (aligned).
const CHARACTER_INFO_BYTES: usize = 4 + 16 + 16 + 4 + 4;

impl Font {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Font> {
        let mut r = Reader::new(data, big_endian);
        let name = r.aligned_string()?;
        let line_spacing = r.f32()?;
        let default_material = PPtr::read(&mut r)?;
        let font_size = r.f32()?;
        let texture = PPtr::read(&mut r)?;
        r.align(4)?;
        let _ascii_start_offset = r.i32()?;
        let _tracking = r.f32()?;
        let _character_spacing = r.i32()?;
        let _character_padding = r.i32()?;
        let _convert_case = r.i32()?;
        let character_rects = r.count(CHARACTER_INFO_BYTES)?;
        for _ in 0..character_rects {
            r.bytes(4 + 16 + 16 + 4 + 1)?;
            r.align(4)?;
        }
        let kerning_pairs = r.count(8)?;
        r.bytes(kerning_pairs * 8)?;
        let pixel_scale = r.f32()?;
        let len = r.count(1)?;
        let font_data = r.bytes(len)?.to_vec();
        r.align(4)?;
        let ascent = r.f32()?;
        let descent = r.f32()?;
        let default_style = r.u32()?;
        let n = r.count(4)?;
        let mut font_names = Vec::with_capacity(n);
        for _ in 0..n {
            font_names.push(r.aligned_string()?);
        }
        let n = r.count(12)?;
        let mut fallback_fonts = Vec::with_capacity(n);
        for _ in 0..n {
            fallback_fonts.push(PPtr::read(&mut r)?);
        }
        let rendering_mode = r.i32()?;
        let use_legacy_bounds = r.u8()? != 0;
        let round_advance = r.u8()? != 0;
        // Stored with or without padding after the flags.
        if r.pos() < data.len() {
            r.align(4)?;
        }
        at_end(&r, data)?;
        Ok(Font {
            name,
            line_spacing,
            default_material,
            font_size,
            texture,
            character_rects,
            kerning_pairs,
            pixel_scale,
            data: font_data,
            ascent,
            descent,
            default_style,
            font_names,
            fallback_fonts,
            rendering_mode,
            use_legacy_bounds,
            round_advance,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pptr(b: &mut Vec<u8>, path: i64) {
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&path.to_le_bytes());
    }

    fn string(b: &mut Vec<u8>, s: &[u8]) {
        b.extend_from_slice(&(s.len() as u32).to_le_bytes());
        b.extend_from_slice(s);
        while b.len() % 4 != 0 {
            b.push(0);
        }
    }

    fn font() -> Vec<u8> {
        let mut b = Vec::new();
        string(&mut b, b"Aller_Rg");
        b.extend_from_slice(&1.2f32.to_le_bytes());
        pptr(&mut b, 4);
        b.extend_from_slice(&16f32.to_le_bytes());
        pptr(&mut b, 5);
        for _ in 0..5 {
            b.extend_from_slice(&0u32.to_le_bytes());
        }
        b.extend_from_slice(&1u32.to_le_bytes()); // one character rect
        b.extend_from_slice(&[0; 41]);
        b.extend_from_slice(&[0; 3]);
        b.extend_from_slice(&1u32.to_le_bytes()); // one kerning pair
        b.extend_from_slice(&[0; 8]);
        b.extend_from_slice(&0.1f32.to_le_bytes());
        string(&mut b, b"\0\x01\0\0abc"); // the font file
        b.extend_from_slice(&12f32.to_le_bytes());
        b.extend_from_slice(&(-3f32).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        string(&mut b, b"Aller");
        b.extend_from_slice(&1u32.to_le_bytes());
        pptr(&mut b, 9);
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&[1, 0, 0, 0]);
        b
    }

    #[test]
    fn font_reads_to_its_last_byte() {
        let b = font();
        let f = Font::parse(&b, false).unwrap();
        assert_eq!(f.name, "Aller_Rg");
        assert_eq!(f.texture.path_id, 5);
        assert_eq!(f.character_rects, 1);
        assert_eq!(f.kerning_pairs, 1);
        assert_eq!(f.data, b"\0\x01\0\0abc");
        assert_eq!((f.ascent, f.descent), (12.0, -3.0));
        assert_eq!(f.font_names, ["Aller"]);
        assert_eq!(f.fallback_fonts.len(), 1);
        assert!(f.use_legacy_bounds && !f.round_advance);
        // Without the trailing padding too.
        assert!(Font::parse(&b[..b.len() - 2], false).is_ok());
        for len in [0, 20, 60, 100, b.len() - 3] {
            assert!(Font::parse(&b[..len], false).is_err(), "{len}");
        }
        let mut extra = b.clone();
        extra.extend_from_slice(&[0; 4]);
        assert!(Font::parse(&extra, false).is_err());
    }
}
