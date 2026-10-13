//! TrueType composite glyphs, placed as FreeType places them when
//! loading unhinted (*confirmed* on `Aller_Rg`'s U+2026, a composite of
//! three periods: its stored width needs this rule, `docs/formats/ui.md`
//! § Fonts). Each component is loaded scaled, its 2×2 transform applied
//! to the scaled points, then its offset added: the offset in font units
//! scaled with `FT_MulFix`, and with `ROUND_XY_TO_GRID` rounded to whole
//! pixels. `ttf-parser` adds the offsets in font units instead, which
//! moves a component by up to half a pixel.
//!
//! Our own reader of the `glyf` composite record, with checked reads.

use ttf_parser::{Face, Tag};

pub const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
pub const ARGS_ARE_XY_VALUES: u16 = 0x0002;
pub const ROUND_XY_TO_GRID: u16 = 0x0004;
pub const WE_HAVE_A_SCALE: u16 = 0x0008;
pub const MORE_COMPONENTS: u16 = 0x0020;
pub const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
pub const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
pub const SCALED_COMPONENT_OFFSET: u16 = 0x0800;

/// One component of a composite glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Component {
    pub glyph: u16,
    pub flags: u16,
    /// Offset in font units (when `ARGS_ARE_XY_VALUES`), else the two
    /// point numbers to match.
    pub args: [i32; 2],
    /// `[xx, yx, xy, yy]` in 16.16 (x' = xx·x + xy·y, y' = yx·x + yy·y);
    /// identity without a transform.
    pub matrix: [i64; 4],
}

impl Component {
    pub fn has_transform(&self) -> bool {
        self.matrix != IDENTITY
    }
}

pub const IDENTITY: [i64; 4] = [0x10000, 0, 0, 0x10000];

fn be16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(at)?, *d.get(at + 1)?]))
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *d.get(at)?,
        *d.get(at + 1)?,
        *d.get(at + 2)?,
        *d.get(at + 3)?,
    ]))
}

/// F2Dot14 to 16.16.
fn f2dot14(v: u16) -> i64 {
    i64::from(v as i16) << 2
}

/// The glyph's `glyf` record, if the font has TrueType outlines.
fn glyph_data<'a>(face: &Face<'a>, glyph: u16) -> Option<&'a [u8]> {
    let raw = face.raw_face();
    let head = raw.table(Tag::from_bytes(b"head"))?;
    let loca = raw.table(Tag::from_bytes(b"loca"))?;
    let glyf = raw.table(Tag::from_bytes(b"glyf"))?;
    let long = be16(head, 50)? != 0;
    let i = usize::from(glyph);
    let (from, to) = if long {
        (be32(loca, i * 4)? as usize, be32(loca, i * 4 + 4)? as usize)
    } else {
        (
            usize::from(be16(loca, i * 2)?) * 2,
            usize::from(be16(loca, i * 2 + 2)?) * 2,
        )
    };
    glyf.get(from..to)
}

/// The components of a composite glyph; `None` for a simple or empty
/// glyph, a font without `glyf`, or a record that does not parse.
pub fn components(face: &Face, glyph: u16) -> Option<Vec<Component>> {
    let d = glyph_data(face, glyph)?;
    if d.len() < 10 || (be16(d, 0)? as i16) >= 0 {
        return None;
    }
    let mut out = Vec::new();
    let mut at = 10;
    loop {
        let flags = be16(d, at)?;
        let glyph = be16(d, at + 2)?;
        at += 4;
        let xy = flags & ARGS_ARE_XY_VALUES != 0;
        let args = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            let (a, b) = (be16(d, at)?, be16(d, at + 2)?);
            at += 4;
            if xy {
                [i32::from(a as i16), i32::from(b as i16)]
            } else {
                [i32::from(a), i32::from(b)]
            }
        } else {
            let (a, b) = (*d.get(at)?, *d.get(at + 1)?);
            at += 2;
            if xy {
                [i32::from(a as i8), i32::from(b as i8)]
            } else {
                [i32::from(a), i32::from(b)]
            }
        };
        let matrix = if flags & WE_HAVE_A_SCALE != 0 {
            let s = f2dot14(be16(d, at)?);
            at += 2;
            [s, 0, 0, s]
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            let m = [f2dot14(be16(d, at)?), 0, 0, f2dot14(be16(d, at + 2)?)];
            at += 4;
            m
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            let m = [
                f2dot14(be16(d, at)?),
                f2dot14(be16(d, at + 2)?),
                f2dot14(be16(d, at + 4)?),
                f2dot14(be16(d, at + 6)?),
            ];
            at += 8;
            m
        } else {
            IDENTITY
        };
        out.push(Component {
            glyph,
            flags,
            args,
            matrix,
        });
        if flags & MORE_COMPONENTS == 0 || out.len() > 1024 {
            break;
        }
    }
    Some(out)
}
