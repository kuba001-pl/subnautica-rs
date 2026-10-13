//! TextMesh Pro fonts (M9u3, `docs/formats/ui.md` § Fonts).
//!
//! Every font asset of the game is dynamic: TMP asks Unity's `FontEngine`
//! (FreeType inside) for each glyph the first time a text needs it, at the
//! asset's sampling point size, and packs its distance field into the
//! asset's atlas. This module does the same from the font file's bytes:
//! - [`FontFile::face_info`]: the face's metrics as TMP stores them in
//!   `m_FaceInfo` (the stored values are what the game uses; this is the
//!   check that we read the font the same way);
//! - [`FontFile::glyph`]: a glyph's metrics, by FreeType's unhinted
//!   fixed-point scaling ([`scale`]);
//! - [`FontFile::render`]: its anti-aliased coverage ([`raster`]) and its
//!   distance field ([`sdf`]);
//! - [`atlas::DynamicAtlas`]: the glyphs packed as TMP packs them.

pub mod atlas;
pub mod composite;
pub mod raster;
pub mod scale;
pub mod sdf;

use ttf_parser::{Face, GlyphId, OutlineBuilder};

pub use atlas::{Added, DynamicAtlas, Glyph, GlyphRect, Packer};
use composite::{ARGS_ARE_XY_VALUES, ROUND_XY_TO_GRID, SCALED_COMPONENT_OFFSET};
pub use raster::Coverage;
use scale::{div_fix, mul_fix};

/// `UnityEngine.TextCore.FaceInfo`, as stored in a font asset.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceInfo {
    pub face_index: i32,
    pub family_name: String,
    pub style_name: String,
    pub point_size: i32,
    pub scale: f32,
    pub line_height: f32,
    pub ascent_line: f32,
    pub cap_line: f32,
    pub mean_line: f32,
    pub baseline: f32,
    pub descent_line: f32,
    pub superscript_offset: f32,
    pub superscript_size: f32,
    pub subscript_offset: f32,
    pub subscript_size: f32,
    pub underline_offset: f32,
    pub underline_thickness: f32,
    pub strikethrough_offset: f32,
    pub strikethrough_thickness: f32,
    pub tab_width: f32,
}

impl FaceInfo {
    /// The numeric fields by their stored names, for comparisons.
    pub fn numbers(&self) -> [(&'static str, f32); 18] {
        [
            ("m_PointSize", self.point_size as f32),
            ("m_Scale", self.scale),
            ("m_LineHeight", self.line_height),
            ("m_AscentLine", self.ascent_line),
            ("m_CapLine", self.cap_line),
            ("m_MeanLine", self.mean_line),
            ("m_Baseline", self.baseline),
            ("m_DescentLine", self.descent_line),
            ("m_SuperscriptOffset", self.superscript_offset),
            ("m_SuperscriptSize", self.superscript_size),
            ("m_SubscriptOffset", self.subscript_offset),
            ("m_SubscriptSize", self.subscript_size),
            ("m_UnderlineOffset", self.underline_offset),
            ("m_UnderlineThickness", self.underline_thickness),
            ("m_StrikethroughOffset", self.strikethrough_offset),
            ("m_StrikethroughThickness", self.strikethrough_thickness),
            ("m_TabWidth", self.tab_width),
            ("m_FaceIndex", self.face_index as f32),
        ]
    }
}

/// `UnityEngine.TextCore.GlyphMetrics`, in pixels at the sampling size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlyphMetrics {
    pub width: f32,
    pub height: f32,
    pub bearing_x: f32,
    pub bearing_y: f32,
    pub advance: f32,
}

/// A glyph's outline in pixels at the sampling size (y up), its metrics
/// and the pixel box its bitmap covers.
#[derive(Clone, Debug, PartialEq)]
pub struct ScaledGlyph {
    pub index: u32,
    pub metrics: GlyphMetrics,
    /// The outline flattened into closed polylines.
    pub contours: Vec<Vec<[f64; 2]>>,
    /// Left and bottom pixel of the bitmap (`floor` of the control box).
    pub left: i32,
    pub bottom: i32,
    /// Bitmap size (`ceil(max) - floor(min)`), FreeType's rule.
    pub width: u32,
    pub height: u32,
}

/// A font file parsed.
pub struct FontFile<'a> {
    face: Face<'a>,
}

/// Why a font or glyph could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontError {
    Parse(String),
    NoGlyph(u32),
    /// A glyph box larger than [`MAX_GLYPH_PIXELS`] (bad font data).
    TooLarge(u32),
}

/// The largest glyph bitmap side we make, in pixels (the game's glyphs
/// are under 200 at their sampling sizes).
pub const MAX_GLYPH_PIXELS: i64 = 4096;

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontError::Parse(e) => write!(f, "font file: {e}"),
            FontError::NoGlyph(g) => write!(f, "no glyph {g}"),
            FontError::TooLarge(g) => write!(f, "glyph {g} is larger than {MAX_GLYPH_PIXELS} px"),
        }
    }
}

impl std::error::Error for FontError {}

/// Nested composite glyphs deeper than this are drawn by `ttf-parser`'s
/// placement (no game glyph is nested; a guard against cycles).
const MAX_COMPOSITE_DEPTH: usize = 8;

/// Flattening tolerance, in pixels.
const FLATNESS: f64 = 0.01;
const MAX_STEPS: usize = 256;

/// Collects an outline in 26.6 pixels (FreeType: each point scaled with
/// `FT_MulFix`), flattening curves.
struct Flattener {
    x_scale: i64,
    y_scale: i64,
    /// Composite placements, outermost first: a 16.16 matrix applied to
    /// the scaled point, then an offset in 26.6.
    placements: Vec<([i64; 4], [i64; 2])>,
    contours: Vec<Vec<[f64; 2]>>,
    /// Every point, control points included, for the control box.
    min: [i64; 2],
    max: [i64; 2],
    last: [f64; 2],
}

impl Flattener {
    fn point(&mut self, x: f32, y: f32) -> [f64; 2] {
        let mut sx = mul_fix(x.round() as i64, self.x_scale);
        let mut sy = mul_fix(y.round() as i64, self.y_scale);
        for (m, o) in self.placements.iter().rev() {
            if *m != composite::IDENTITY {
                (sx, sy) = (
                    mul_fix(sx, m[0]) + mul_fix(sy, m[2]),
                    mul_fix(sx, m[1]) + mul_fix(sy, m[3]),
                );
            }
            sx = sx.saturating_add(o[0]);
            sy = sy.saturating_add(o[1]);
        }
        self.min = [self.min[0].min(sx), self.min[1].min(sy)];
        self.max = [self.max[0].max(sx), self.max[1].max(sy)];
        [sx as f64 / 64.0, sy as f64 / 64.0]
    }

    fn push(&mut self, p: [f64; 2]) {
        if let Some(c) = self.contours.last_mut() {
            c.push(p);
        }
        self.last = p;
    }
}

fn steps(deviation: f64) -> usize {
    ((deviation / FLATNESS).sqrt().ceil() as usize).clamp(1, MAX_STEPS)
}

fn len(v: [f64; 2]) -> f64 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

impl OutlineBuilder for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.contours.push(Vec::new());
        self.push(p);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.push(p);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.last;
        let p1 = self.point(x1, y1);
        let p2 = self.point(x, y);
        let d = len([p0[0] - 2.0 * p1[0] + p2[0], p0[1] - 2.0 * p1[1] + p2[1]]) / 4.0;
        let n = steps(d);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            self.push([
                u * u * p0[0] + 2.0 * u * t * p1[0] + t * t * p2[0],
                u * u * p0[1] + 2.0 * u * t * p1[1] + t * t * p2[1],
            ]);
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.last;
        let p1 = self.point(x1, y1);
        let p2 = self.point(x2, y2);
        let p3 = self.point(x, y);
        let d = len([p0[0] - 2.0 * p1[0] + p2[0], p0[1] - 2.0 * p1[1] + p2[1]]).max(len([
            p1[0] - 2.0 * p2[0] + p3[0],
            p1[1] - 2.0 * p2[1] + p3[1],
        ])) * 0.75;
        let n = steps(d);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            let (a, b, c, e) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.push([
                a * p0[0] + b * p1[0] + c * p2[0] + e * p3[0],
                a * p0[1] + b * p1[1] + c * p2[1] + e * p3[1],
            ]);
        }
    }

    fn close(&mut self) {}
}

impl<'a> FontFile<'a> {
    pub fn parse(data: &'a [u8]) -> Result<FontFile<'a>, FontError> {
        let face = Face::parse(data, 0).map_err(|e| FontError::Parse(e.to_string()))?;
        Ok(FontFile { face })
    }

    pub fn units_per_em(&self) -> u16 {
        self.face.units_per_em()
    }

    pub fn glyph_count(&self) -> u16 {
        self.face.number_of_glyphs()
    }

    /// The glyph index of a character (the `cmap`), if the font has it.
    pub fn glyph_index(&self, c: char) -> Option<u32> {
        self.face.glyph_index(c).map(|g| u32::from(g.0))
    }

    /// The name table's string for `id`, preferring Windows Unicode.
    fn name(&self, id: u16) -> Option<String> {
        let names = self.face.names();
        let mut fallback = None;
        for n in names {
            if n.name_id != id {
                continue;
            }
            if let Some(s) = n.to_string() {
                if n.platform_id == ttf_parser::PlatformId::Windows {
                    return Some(s);
                }
                fallback.get_or_insert(s);
            }
        }
        fallback
    }

    /// FreeType's 16.16 scale for a sampling size in pixels (TMP's point
    /// size at 72 dpi): `FT_DivFix(size * 64, units_per_em)`.
    fn scales(&self, point_size: i32) -> i64 {
        div_fix(
            i64::from(point_size) * 64,
            i64::from(self.face.units_per_em()),
        )
    }

    /// The face's metrics at `point_size`, by the rule found against the
    /// game's stored `m_FaceInfo` (`docs/formats/ui.md` § Fonts):
    /// ascent, descent and line height from `hhea` scaled in floating
    /// point; underline from `post` (position minus half the thickness,
    /// FreeType's rule); strikethrough at the mean line / 2.5 with the
    /// underline's thickness; super- and subscript at the ascent and
    /// descent, half size; tab width the space's advance rounded; cap and
    /// mean lines the tops of `H` and `x` (0 without them) — the game's
    /// are hinted, ours are not (*confirmed* differences, logged).
    pub fn face_info(&self, point_size: i32) -> FaceInfo {
        let f = &self.face;
        let upem = f32::from(f.units_per_em());
        let s = point_size as f32 / upem;
        let ascent = f32::from(f.ascender()) * s;
        let descent = f32::from(f.descender()) * s;
        let line_height =
            (f32::from(f.ascender()) - f32::from(f.descender()) + f32::from(f.line_gap())) * s;
        let (underline, thickness) = match f.underline_metrics() {
            Some(m) => (
                f32::from(m.position.saturating_sub(m.thickness / 2)) * s,
                f32::from(m.thickness) * s,
            ),
            None => (0.0, 0.0),
        };
        let top = |c: char| -> f32 {
            self.glyph_index(c)
                .and_then(|g| self.glyph(g, point_size).ok())
                .map_or(0.0, |g| g.metrics.bearing_y)
        };
        let cap_line = top('H');
        let mean_line = top('x');
        let tab = self
            .glyph_index(' ')
            .and_then(|g| f.glyph_hor_advance(GlyphId(g as u16)))
            .map_or(0.0, |a| (f32::from(a) * s).round());
        FaceInfo {
            face_index: 0,
            family_name: self.name(16).or_else(|| self.name(1)).unwrap_or_default(),
            style_name: self.name(17).or_else(|| self.name(2)).unwrap_or_default(),
            point_size,
            scale: 1.0,
            line_height,
            ascent_line: ascent,
            cap_line,
            mean_line,
            baseline: 0.0,
            descent_line: descent,
            superscript_offset: ascent,
            superscript_size: 0.5,
            subscript_offset: descent,
            subscript_size: 0.5,
            underline_offset: underline,
            underline_thickness: thickness,
            strikethrough_offset: mean_line / 2.5,
            strikethrough_thickness: thickness,
            tab_width: tab,
        }
    }

    /// A glyph at `point_size`, unhinted (render mode SDFAA): each outline
    /// point scaled to 26.6 pixels with `FT_MulFix`; the metrics from the
    /// control box of those points and the scaled advance, in 1/64 px.
    pub fn glyph(&self, index: u32, point_size: i32) -> Result<ScaledGlyph, FontError> {
        let gid = u16::try_from(index).map_err(|_| FontError::NoGlyph(index))?;
        if gid >= self.face.number_of_glyphs() {
            return Err(FontError::NoGlyph(index));
        }
        let scale = self.scales(point_size);
        let mut b = Flattener {
            x_scale: scale,
            y_scale: scale,
            placements: Vec::new(),
            contours: Vec::new(),
            min: [i64::MAX, i64::MAX],
            max: [i64::MIN, i64::MIN],
            last: [0.0, 0.0],
        };
        let outlined = self.outline(gid, &mut b, 0);
        let advance = self
            .face
            .glyph_hor_advance(GlyphId(gid))
            .map_or(0, |a| mul_fix(i64::from(a), scale));
        if !outlined || b.min[0] > b.max[0] {
            return Ok(ScaledGlyph {
                index,
                metrics: GlyphMetrics {
                    advance: advance as f32 / 64.0,
                    ..GlyphMetrics::default()
                },
                contours: Vec::new(),
                left: 0,
                bottom: 0,
                width: 0,
                height: 0,
            });
        }
        let [x0, y0] = b.min;
        let [x1, y1] = b.max;
        let metrics = GlyphMetrics {
            width: x1.saturating_sub(x0) as f32 / 64.0,
            height: y1.saturating_sub(y0) as f32 / 64.0,
            bearing_x: x0 as f32 / 64.0,
            bearing_y: y1 as f32 / 64.0,
            advance: advance as f32 / 64.0,
        };
        // FreeType's bitmap box: floor of the minimum, ceil of the maximum.
        let left = x0.div_euclid(64);
        let bottom = y0.div_euclid(64);
        let right = x1.saturating_add(63).div_euclid(64);
        let top = y1.saturating_add(63).div_euclid(64);
        if right.saturating_sub(left) > MAX_GLYPH_PIXELS
            || top.saturating_sub(bottom) > MAX_GLYPH_PIXELS
        {
            return Err(FontError::TooLarge(index));
        }
        Ok(ScaledGlyph {
            index,
            metrics,
            contours: b.contours,
            left: left as i32,
            bottom: bottom as i32,
            width: (right - left) as u32,
            height: (top - bottom) as u32,
        })
    }

    /// Emits a glyph's outline: a composite one component by component,
    /// each placed by FreeType's rule ([`composite`]); a simple one, or a
    /// composite placed by point matching, through `ttf-parser`.
    fn outline(&self, gid: u16, b: &mut Flattener, depth: usize) -> bool {
        let parts = (depth < MAX_COMPOSITE_DEPTH)
            .then(|| composite::components(&self.face, gid))
            .flatten();
        let Some(parts) = parts.filter(|p| p.iter().all(|c| c.flags & ARGS_ARE_XY_VALUES != 0))
        else {
            return self.face.outline_glyph(GlyphId(gid), b).is_some();
        };
        let mut any = false;
        for c in parts {
            let (mut dx, mut dy) = (i64::from(c.args[0]), i64::from(c.args[1]));
            if c.flags & SCALED_COMPONENT_OFFSET != 0 && c.has_transform() {
                let m = c.matrix;
                (dx, dy) = (
                    mul_fix(dx, m[0]) + mul_fix(dy, m[2]),
                    mul_fix(dx, m[1]) + mul_fix(dy, m[3]),
                );
            }
            let mut ox = mul_fix(dx, b.x_scale);
            let mut oy = mul_fix(dy, b.y_scale);
            if c.flags & ROUND_XY_TO_GRID != 0 {
                ox = (ox + 32).div_euclid(64) * 64;
                oy = (oy + 32).div_euclid(64) * 64;
            }
            b.placements.push((c.matrix, [ox, oy]));
            any |= self.outline(c.glyph, b, depth + 1);
            b.placements.pop();
        }
        any
    }

    /// The glyph's distance field with `padding` texels around its bitmap:
    /// `(width + 2·padding) × (height + 2·padding)` bytes, bottom row
    /// first (Unity's texture order). See [`sdf`] for the encoding.
    pub fn render(&self, glyph: &ScaledGlyph, padding: u32) -> sdf::Field {
        sdf::glyph_field(glyph, padding)
    }
}

#[cfg(test)]
mod tests;
