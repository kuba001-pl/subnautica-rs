//! uGUI graphic meshes (M9u4b, `docs/formats/ui.md` § Graphic meshes):
//! what `Image.OnPopulateMesh` and `RawImage.OnPopulateMesh` build, in
//! the node's own space (its pivot at the origin), with Unity's vertex
//! order and triangles.
//!
//! The rules are written from the IL of `UnityEngine.UI.dll` (2019.4):
//! `Image`'s type switch (simple, or the sprite's own mesh; sliced; tiled;
//! filled), `GetDrawingDimensions` (the sprite size rounded to whole
//! pixels, its trimmed margin, preserve aspect), `GetAdjustedBorders`,
//! the tiled path's choice between a mesh of tiles and texture repeat and
//! its 65,000-vertex cap, `RadialCut` (fill below 0.001 draws nothing,
//! above 0.999 not inverted draws whole). The code is ours.

use crate::layout::{Rect, V2};

/// A UI vertex: position in the node's space, colour, texture coordinate
/// and the second texture coordinate (`uv1`, 0 for images; TMP's packed
/// face UV and scale for text, M9u5a).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub pos: V2,
    pub color: [u8; 4],
    pub uv: V2,
    pub uv1: V2,
}

/// A graphic's mesh: triangles as index triples.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    fn quad(&mut self, pos: [V2; 4], uv: [V2; 4], color: [u8; 4]) {
        let s = self.vertices.len() as u32;
        for k in 0..4 {
            self.vertices.push(Vertex {
                pos: pos[k],
                color,
                uv: uv[k],
                uv1: [0.0, 0.0],
            });
        }
        self.indices
            .extend_from_slice(&[s, s + 1, s + 2, s + 2, s + 3, s]);
    }

    /// `AddQuad(min, max, uvMin, uvMax)`: bottom left, top left, top
    /// right, bottom right.
    fn rect_quad(&mut self, min: V2, max: V2, uv_min: V2, uv_max: V2, color: [u8; 4]) {
        self.quad(
            [min, [min[0], max[1]], max, [max[0], min[1]]],
            [
                uv_min,
                [uv_min[0], uv_max[1]],
                uv_max,
                [uv_max[0], uv_min[1]],
            ],
            color,
        );
    }
}

/// `Graphic.OnPopulateMesh`: the rect as one quad, UVs 0 to 1, the colour
/// as `Color32` (the game's own graphics that do not build a mesh, as
/// `uGUI_CircularBar`, and an `Image` without a sprite).
pub fn graphic_quad(rect: Rect, color: [f32; 4]) -> MeshData {
    let mut m = MeshData::default();
    m.rect_quad(
        [rect.x, rect.y],
        [rect.x + rect.width, rect.y + rect.height],
        [0.0, 0.0],
        [1.0, 1.0],
        color32(color),
    );
    m
}

/// Unity's `Color` → `Color32` conversion (`UnityEngine.CoreModule`):
/// `(byte)Mathf.Round(Mathf.Clamp01(c) × 255)`. `Mathf.Round` is
/// `Math.Round`, which rounds half to even (M9u6b1; before, half away
/// from zero).
pub fn color32(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round_ties_even() as u8)
}

/// What an `Image` needs of its sprite (Unity's `DataUtility` values).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteInfo {
    /// `sprite.rect` size, pixels.
    pub size: V2,
    /// Left, bottom, right, top, pixels.
    pub border: [f32; 4],
    pub pixels_per_unit: f32,
    /// `DataUtility.GetOuterUV`: the packed rectangle in its texture.
    pub outer_uv: [f32; 4],
    /// `DataUtility.GetInnerUV`: the same inside the borders.
    pub inner_uv: [f32; 4],
    /// `DataUtility.GetPadding`: the trimmed margin, left, bottom, right,
    /// top, pixels.
    pub padding: [f32; 4],
    /// `Sprite.packed`.
    pub packed: bool,
    /// The texture's wrap mode is Repeat.
    pub repeat: bool,
}

impl SpriteInfo {
    /// From a sprite's rect and borders and where it sits in its texture
    /// (`textureRect`, `textureRectOffset`, both pixels).
    ///
    /// Outer UV and padding are *confirmed* by their definition (the
    /// texture rectangle; the margin a tight packing trims). The inner UV
    /// is a *hypothesis*: the texture rectangle inset by each border less
    /// the trimmed margin. Every bordered sprite of the game is
    /// rectangle-packed or unpacked (no margin), where it is exact.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rect_size: V2,
        border: [f32; 4],
        pixels_per_unit: f32,
        texture_rect: [f32; 4],
        texture_rect_offset: V2,
        texture_size: V2,
        packed: bool,
        repeat: bool,
    ) -> SpriteInfo {
        let [x, y, w, h] = texture_rect;
        let [tw, th] = [texture_size[0].max(1.0), texture_size[1].max(1.0)];
        let padding = [
            texture_rect_offset[0],
            texture_rect_offset[1],
            rect_size[0] - w - texture_rect_offset[0],
            rect_size[1] - h - texture_rect_offset[1],
        ];
        let inset = |k: usize| (border[k] - padding[k]).max(0.0);
        SpriteInfo {
            size: rect_size,
            border,
            pixels_per_unit,
            outer_uv: [x / tw, y / th, (x + w) / tw, (y + h) / th],
            inner_uv: [
                (x + inset(0)) / tw,
                (y + inset(1)) / th,
                (x + w - inset(2)) / tw,
                (y + h - inset(3)) / th,
            ],
            padding,
            packed,
            repeat,
        }
    }

    fn has_border(&self) -> bool {
        self.border.iter().any(|&b| b != 0.0)
    }
}

/// `Image`'s fields that shape its mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageParams {
    pub sprite: Option<SpriteInfo>,
    /// 0 simple, 1 sliced, 2 tiled, 3 filled.
    pub kind: i32,
    pub preserve_aspect: bool,
    pub fill_center: bool,
    /// 0 horizontal, 1 vertical, 2 radial 90, 3 radial 180, 4 radial 360.
    pub fill_method: i32,
    pub fill_amount: f32,
    pub fill_clockwise: bool,
    pub fill_origin: i32,
    pub use_sprite_mesh: bool,
    pub pixels_per_unit_multiplier: f32,
    pub color: [f32; 4],
}

/// What was built, for the logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MeshKind {
    /// No sprite: `Graphic`'s quad.
    Plain,
    Simple,
    /// `useSpriteMesh`: the sprite's own mesh (not ported: drawn simple).
    SpriteMeshAsSimple,
    Sliced,
    /// Sliced without borders: simple.
    SlicedAsSimple,
    TiledQuads,
    TiledRepeat,
    Filled,
}

/// `Image.OnPopulateMesh`. `rect` is the node's rect (world-space canvases
/// are not pixel-adjusted), `pivot` its pivot, `reference_ppu` the
/// canvas's reference pixels per unit (100).
pub fn image_mesh(
    rect: Rect,
    pivot: V2,
    img: &ImageParams,
    reference_ppu: f32,
) -> (MeshKind, MeshData) {
    let color = color32(img.color);
    let mut m = MeshData::default();
    let Some(sprite) = img.sprite else {
        m.rect_quad(
            [rect.x, rect.y],
            [rect.x + rect.width, rect.y + rect.height],
            [0.0, 0.0],
            [1.0, 1.0],
            color,
        );
        return (MeshKind::Plain, m);
    };
    let ppu = sprite.pixels_per_unit / reference_ppu;
    let multiplied = ppu * img.pixels_per_unit_multiplier;
    let kind = match img.kind {
        1 if sprite.has_border() => {
            sliced(&mut m, rect, &sprite, multiplied, img.fill_center, color);
            MeshKind::Sliced
        }
        1 => {
            simple(&mut m, rect, pivot, &sprite, false, color);
            MeshKind::SlicedAsSimple
        }
        2 => tiled(&mut m, rect, &sprite, multiplied, img.fill_center, color),
        3 => {
            filled(&mut m, rect, pivot, &sprite, img, color);
            MeshKind::Filled
        }
        _ if img.use_sprite_mesh => {
            simple(&mut m, rect, pivot, &sprite, img.preserve_aspect, color);
            MeshKind::SpriteMeshAsSimple
        }
        _ => {
            simple(&mut m, rect, pivot, &sprite, img.preserve_aspect, color);
            MeshKind::Simple
        }
    };
    (kind, m)
}

/// `RawImage.OnPopulateMesh`: one quad with `uv_rect` (x, y, width,
/// height), if there is a texture.
pub fn raw_image_mesh(
    rect: Rect,
    color: [f32; 4],
    uv_rect: [f32; 4],
    has_texture: bool,
) -> MeshData {
    let mut m = MeshData::default();
    if has_texture {
        m.rect_quad(
            [rect.x, rect.y],
            [rect.x + rect.width, rect.y + rect.height],
            [uv_rect[0], uv_rect[1]],
            [uv_rect[0] + uv_rect[2], uv_rect[1] + uv_rect[3]],
            color32(color),
        );
    }
    m
}

/// `PreserveSpriteAspectRatio`.
fn preserve_aspect(rect: &mut Rect, pivot: V2, size: V2) {
    let sprite_ratio = size[0] / size[1];
    let rect_ratio = rect.width / rect.height;
    if sprite_ratio > rect_ratio {
        let old = rect.height;
        rect.height = rect.width * (1.0 / sprite_ratio);
        rect.y += (old - rect.height) * pivot[1];
    } else {
        let old = rect.width;
        rect.width = rect.height * sprite_ratio;
        rect.x += (old - rect.width) * pivot[0];
    }
}

/// `GetDrawingDimensions`: x min, y min, x max, y max.
fn drawing_dimensions(rect: Rect, pivot: V2, s: &SpriteInfo, keep_aspect: bool) -> [f32; 4] {
    let mut r = rect;
    let w = s.size[0].round();
    let h = s.size[1].round();
    let p = s.padding;
    let v = [p[0] / w, p[1] / h, (w - p[2]) / w, (h - p[3]) / h];
    if keep_aspect && s.size[0] * s.size[0] + s.size[1] * s.size[1] > 0.0 {
        preserve_aspect(&mut r, pivot, s.size);
    }
    [
        r.x + r.width * v[0],
        r.y + r.height * v[1],
        r.x + r.width * v[2],
        r.y + r.height * v[3],
    ]
}

fn simple(
    m: &mut MeshData,
    rect: Rect,
    pivot: V2,
    s: &SpriteInfo,
    keep_aspect: bool,
    color: [u8; 4],
) {
    let v = drawing_dimensions(rect, pivot, s, keep_aspect);
    let uv = s.outer_uv;
    m.rect_quad(
        [v[0], v[1]],
        [v[2], v[3]],
        [uv[0], uv[1]],
        [uv[2], uv[3]],
        color,
    );
}

/// `GetAdjustedBorders`: borders scaled when the rect is smaller than
/// both together. `rect` is both the original and the adjusted rect here
/// (no pixel adjustment on world-space canvases).
fn adjusted_borders(mut b: [f32; 4], rect: Rect) -> [f32; 4] {
    let size = rect.size();
    for axis in 0..2 {
        let combined = b[axis] + b[axis + 2];
        if size[axis] < combined && combined != 0.0 {
            let ratio = size[axis] / combined;
            b[axis] *= ratio;
            b[axis + 2] *= ratio;
        }
    }
    b
}

fn sliced(
    m: &mut MeshData,
    rect: Rect,
    s: &SpriteInfo,
    ppu: f32,
    fill_center: bool,
    color: [u8; 4],
) {
    let (outer, inner) = (s.outer_uv, s.inner_uv);
    let b = adjusted_borders(s.border.map(|v| v / ppu), rect);
    let p = s.padding.map(|v| v / ppu);
    let xs = [p[0], b[0], rect.width - b[2], rect.width - p[2]].map(|x| x + rect.x);
    let ys = [p[1], b[1], rect.height - b[3], rect.height - p[3]].map(|y| y + rect.y);
    let us = [outer[0], inner[0], inner[2], outer[2]];
    let vs = [outer[1], inner[1], inner[3], outer[3]];
    for x in 0..3 {
        for y in 0..3 {
            if !fill_center && x == 1 && y == 1 {
                continue;
            }
            m.rect_quad(
                [xs[x], ys[y]],
                [xs[x + 1], ys[y + 1]],
                [us[x], vs[y]],
                [us[x + 1], vs[y + 1]],
                color,
            );
        }
    }
}

const MAX_VERTICES: f64 = 65000.0;

fn tiled(
    m: &mut MeshData,
    rect: Rect,
    s: &SpriteInfo,
    ppu: f32,
    fill_center: bool,
    color: [u8; 4],
) -> MeshKind {
    let (outer, inner) = (s.outer_uv, s.inner_uv);
    let mut tile_w = (s.size[0] - s.border[0] - s.border[2]) / ppu;
    let mut tile_h = (s.size[1] - s.border[1] - s.border[3]) / ppu;
    let b = adjusted_borders(s.border.map(|v| v / ppu), rect);
    let uv_min = [inner[0], inner[1]];
    let uv_max = [inner[2], inner[3]];
    let (x_min, x_max) = (b[0], rect.width - b[2]);
    let (y_min, y_max) = (b[1], rect.height - b[3]);
    let at = |x: f32, y: f32| [x + rect.x, y + rect.y];
    if tile_w <= 0.0 {
        tile_w = x_max - x_min;
    }
    if tile_h <= 0.0 {
        tile_h = y_max - y_min;
    }
    let border = s.has_border();
    if !(border || s.packed || !s.repeat) {
        // Texture repeat: one quad, the UVs scaled by the tile count.
        if fill_center {
            let scale = [(x_max - x_min) / tile_w, (y_max - y_min) / tile_h];
            m.rect_quad(
                at(x_min, y_min),
                at(x_max, y_max),
                [uv_min[0] * scale[0], uv_min[1] * scale[1]],
                [uv_max[0] * scale[0], uv_max[1] * scale[1]],
                color,
            );
        }
        return MeshKind::TiledRepeat;
    }
    let ceil = |a: f32, t: f32| ((a / t) as f64).ceil() as i64;
    let (mut nw, mut nh): (i64, i64);
    if fill_center {
        nw = ceil(x_max - x_min, tile_w);
        nh = ceil(y_max - y_min, tile_h);
        let verts = if border {
            (nw as f64 + 2.0) * (nh as f64 + 2.0) * 4.0
        } else {
            nw as f64 * nh as f64 * 4.0
        };
        if verts > MAX_VERTICES {
            let max_tiles = MAX_VERTICES / 4.0;
            let ratio = if border {
                (nw as f64 + 2.0) / (nh as f64 + 2.0)
            } else {
                nw as f64 / nh as f64
            };
            let mut tw = (max_tiles / ratio).sqrt();
            let mut th = tw * ratio;
            if border {
                tw -= 2.0;
                th -= 2.0;
            }
            nw = tw.floor() as i64;
            nh = th.floor() as i64;
            tile_w = (x_max - x_min) / nw as f32;
            tile_h = (y_max - y_min) / nh as f32;
        }
    } else if border {
        nw = ceil(x_max - x_min, tile_w);
        nh = ceil(y_max - y_min, tile_h);
        let verts = (nh as f64 + nw as f64 + 2.0) * 2.0 * 4.0;
        if verts > MAX_VERTICES {
            let max_tiles = MAX_VERTICES / 4.0;
            let ratio = nw as f64 / nh as f64;
            let tw = (max_tiles - 4.0) / (2.0 * (1.0 + ratio));
            let th = tw * ratio;
            nw = tw.floor() as i64;
            nh = th.floor() as i64;
            tile_w = (x_max - x_min) / nw as f32;
            tile_h = (y_max - y_min) / nh as f32;
        }
    } else {
        nw = 0;
        nh = 0;
    }
    // Bad input must not loop for ever.
    let (nw, nh) = (nw.clamp(0, 1 << 16), nh.clamp(0, 1 << 16));
    if fill_center {
        for j in 0..nh {
            let y1 = y_min + j as f32 * tile_h;
            let mut y2 = y_min + (j + 1) as f32 * tile_h;
            let mut clipped = uv_max;
            if y2 > y_max {
                clipped[1] = uv_min[1] + (uv_max[1] - uv_min[1]) * (y_max - y1) / (y2 - y1);
                y2 = y_max;
            }
            clipped[0] = uv_max[0];
            for i in 0..nw {
                let x1 = x_min + i as f32 * tile_w;
                let mut x2 = x_min + (i + 1) as f32 * tile_w;
                if x2 > x_max {
                    clipped[0] = uv_min[0] + (uv_max[0] - uv_min[0]) * (x_max - x1) / (x2 - x1);
                    x2 = x_max;
                }
                m.rect_quad(at(x1, y1), at(x2, y2), uv_min, clipped, color);
            }
        }
    }
    if border {
        let mut clipped = uv_max;
        for j in 0..nh {
            let y1 = y_min + j as f32 * tile_h;
            let mut y2 = y_min + (j + 1) as f32 * tile_h;
            if y2 > y_max {
                clipped[1] = uv_min[1] + (uv_max[1] - uv_min[1]) * (y_max - y1) / (y2 - y1);
                y2 = y_max;
            }
            m.rect_quad(
                at(0.0, y1),
                at(x_min, y2),
                [outer[0], uv_min[1]],
                [uv_min[0], clipped[1]],
                color,
            );
            m.rect_quad(
                at(x_max, y1),
                at(rect.width, y2),
                [uv_max[0], uv_min[1]],
                [outer[2], clipped[1]],
                color,
            );
        }
        let mut clipped = uv_max;
        for i in 0..nw {
            let x1 = x_min + i as f32 * tile_w;
            let mut x2 = x_min + (i + 1) as f32 * tile_w;
            if x2 > x_max {
                clipped[0] = uv_min[0] + (uv_max[0] - uv_min[0]) * (x_max - x1) / (x2 - x1);
                x2 = x_max;
            }
            m.rect_quad(
                at(x1, 0.0),
                at(x2, y_min),
                [uv_min[0], outer[1]],
                [clipped[0], uv_min[1]],
                color,
            );
            m.rect_quad(
                at(x1, y_max),
                at(x2, rect.height),
                [uv_min[0], uv_max[1]],
                [clipped[0], outer[3]],
                color,
            );
        }
        m.rect_quad(
            at(0.0, 0.0),
            at(x_min, y_min),
            [outer[0], outer[1]],
            uv_min,
            color,
        );
        m.rect_quad(
            at(x_max, 0.0),
            at(rect.width, y_min),
            [uv_max[0], outer[1]],
            [outer[2], uv_min[1]],
            color,
        );
        m.rect_quad(
            at(0.0, y_max),
            at(x_min, rect.height),
            [outer[0], uv_max[1]],
            [uv_min[0], outer[3]],
            color,
        );
        m.rect_quad(
            at(x_max, y_max),
            at(rect.width, rect.height),
            uv_max,
            [outer[2], outer[3]],
            color,
        );
    }
    MeshKind::TiledQuads
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// `RadialCut(xy, cos, sin, invert, corner)` on one set of four points.
fn radial_cut_points(xy: &mut [V2; 4], mut cos: f32, mut sin: f32, invert: bool, corner: usize) {
    let i0 = corner;
    let i1 = (corner + 1) % 4;
    let i2 = (corner + 2) % 4;
    let i3 = (corner + 3) % 4;
    if corner & 1 == 1 {
        if sin > cos {
            cos /= sin;
            sin = 1.0;
            if invert {
                xy[i1][0] = lerp(xy[i0][0], xy[i2][0], cos);
                xy[i2][0] = xy[i1][0];
            }
        } else if cos > sin {
            sin /= cos;
            cos = 1.0;
            if !invert {
                xy[i2][1] = lerp(xy[i0][1], xy[i2][1], sin);
                xy[i3][1] = xy[i2][1];
            }
        } else {
            cos = 1.0;
            sin = 1.0;
        }
        if !invert {
            xy[i3][0] = lerp(xy[i0][0], xy[i2][0], cos);
        } else {
            xy[i1][1] = lerp(xy[i0][1], xy[i2][1], sin);
        }
    } else {
        if cos > sin {
            sin /= cos;
            cos = 1.0;
            if !invert {
                xy[i1][1] = lerp(xy[i0][1], xy[i2][1], sin);
                xy[i2][1] = xy[i1][1];
            }
        } else if sin > cos {
            cos /= sin;
            sin = 1.0;
            if invert {
                xy[i2][0] = lerp(xy[i0][0], xy[i2][0], cos);
                xy[i3][0] = xy[i2][0];
            }
        } else {
            cos = 1.0;
            sin = 1.0;
        }
        if invert {
            xy[i3][1] = lerp(xy[i0][1], xy[i2][1], sin);
        } else {
            xy[i1][0] = lerp(xy[i0][0], xy[i2][0], cos);
        }
    }
}

/// `RadialCut(xy, uv, fill, invert, corner)`: false draws nothing.
fn radial_cut(
    xy: &mut [V2; 4],
    uv: &mut [V2; 4],
    fill: f32,
    mut invert: bool,
    corner: usize,
) -> bool {
    if fill < 0.001 {
        return false;
    }
    if corner & 1 == 1 {
        invert = !invert;
    }
    if !invert && fill > 0.999 {
        return true;
    }
    let mut angle = fill.clamp(0.0, 1.0);
    if invert {
        angle = 1.0 - angle;
    }
    angle *= std::f32::consts::FRAC_PI_2;
    let (cos, sin) = (angle.cos(), angle.sin());
    radial_cut_points(xy, cos, sin, invert, corner);
    radial_cut_points(uv, cos, sin, invert, corner);
    true
}

/// The four corners of the part `[fx0, fx1] × [fy0, fy1]` of `v` (and the
/// same of the UVs): bottom left, top left, top right, bottom right.
fn part(v: [f32; 4], f: [f32; 4]) -> [V2; 4] {
    let x0 = lerp(v[0], v[2], f[0]);
    let x1 = lerp(v[0], v[2], f[1]);
    let y0 = lerp(v[1], v[3], f[2]);
    let y1 = lerp(v[1], v[3], f[3]);
    [[x0, y0], [x0, y1], [x1, y1], [x1, y0]]
}

fn filled(
    m: &mut MeshData,
    rect: Rect,
    pivot: V2,
    s: &SpriteInfo,
    img: &ImageParams,
    color: [u8; 4],
) {
    let amount = img.fill_amount;
    if amount < 0.001 {
        return;
    }
    let mut v = drawing_dimensions(rect, pivot, s, img.preserve_aspect);
    let [mut tx0, mut ty0, mut tx1, mut ty1] = s.outer_uv;
    match img.fill_method {
        0 => {
            let fill = (tx1 - tx0) * amount;
            if img.fill_origin == 1 {
                v[0] = v[2] - (v[2] - v[0]) * amount;
                tx0 = tx1 - fill;
            } else {
                v[2] = v[0] + (v[2] - v[0]) * amount;
                tx1 = tx0 + fill;
            }
        }
        1 => {
            let fill = (ty1 - ty0) * amount;
            if img.fill_origin == 1 {
                v[1] = v[3] - (v[3] - v[1]) * amount;
                ty0 = ty1 - fill;
            } else {
                v[3] = v[1] + (v[3] - v[1]) * amount;
                ty1 = ty0 + fill;
            }
        }
        _ => {}
    }
    let t = [tx0, ty0, tx1, ty1];
    let whole = |m: &mut MeshData| {
        m.quad(
            part(v, [0.0, 1.0, 0.0, 1.0]),
            part(t, [0.0, 1.0, 0.0, 1.0]),
            color,
        );
    };
    if amount >= 1.0 || img.fill_method < 2 {
        whole(m);
        return;
    }
    let origin = img.fill_origin.clamp(0, 3) as usize;
    match img.fill_method {
        2 => {
            let mut xy = part(v, [0.0, 1.0, 0.0, 1.0]);
            let mut uv = part(t, [0.0, 1.0, 0.0, 1.0]);
            if radial_cut(&mut xy, &mut uv, amount, img.fill_clockwise, origin) {
                m.quad(xy, uv, color);
            }
        }
        3 => {
            for side in 0..2 {
                let even = usize::from(origin > 1);
                let f = if origin == 0 || origin == 2 {
                    if side == even {
                        [0.0, 0.5, 0.0, 1.0]
                    } else {
                        [0.5, 1.0, 0.0, 1.0]
                    }
                } else if side == even {
                    [0.0, 1.0, 0.5, 1.0]
                } else {
                    [0.0, 1.0, 0.0, 0.5]
                };
                let mut xy = part(v, f);
                let mut uv = part(t, f);
                let val = if img.fill_clockwise {
                    amount * 2.0 - side as f32
                } else {
                    amount * 2.0 - (1 - side) as f32
                };
                if radial_cut(
                    &mut xy,
                    &mut uv,
                    val.clamp(0.0, 1.0),
                    img.fill_clockwise,
                    (side + origin + 3) % 4,
                ) {
                    m.quad(xy, uv, color);
                }
            }
        }
        4 => {
            for corner in 0..4 {
                let fx = if corner < 2 { [0.0, 0.5] } else { [0.5, 1.0] };
                let fy = if corner == 0 || corner == 3 {
                    [0.0, 0.5]
                } else {
                    [0.5, 1.0]
                };
                let f = [fx[0], fx[1], fy[0], fy[1]];
                let mut xy = part(v, f);
                let mut uv = part(t, f);
                let k = ((corner + origin) % 4) as f32;
                let val = if img.fill_clockwise {
                    amount * 4.0 - k
                } else {
                    amount * 4.0 - (3.0 - k)
                };
                if radial_cut(
                    &mut xy,
                    &mut uv,
                    val.clamp(0.0, 1.0),
                    img.fill_clockwise,
                    (corner + 2) % 4,
                ) {
                    m.quad(xy, uv, color);
                }
            }
        }
        _ => whole(m),
    }
}

#[cfg(test)]
mod tests;
