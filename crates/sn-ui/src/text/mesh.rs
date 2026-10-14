//! A laid-out text's vertices (M9u5a): what `TextMeshProUGUI` puts in its
//! mesh and its sub-meshes, ported from the decompiled TMP (read only; the
//! code is ours):
//! - `SaveGlyphVertexInfo`: each character's atlas UVs, its glyph rect
//!   grown by the padding and style padding its quad was made with (the
//!   style padding only for an SDF material), and its colour, whose alpha
//!   is at most the text colour's;
//! - the alignment pass of `GenerateTextMesh`: `uv2` (our `uv1`), the
//!   face-texture UV packed by `PackUV` and the SDF scale (the
//!   character's scale × `1 − charWidthAdjDelta`, negative for bold, ×
//!   the text's `|lossyScale.y|`);
//! - `FillCharacterVertexBuffers`: one quad per visible character, in the
//!   mesh of its font's material (the text's own font first, then each
//!   fallback font in the order first met, as TMP's sub-meshes);
//! - sprites (M9u6d1, `SaveSpriteVertexInfo`, `FillSpriteVertexBuffers`):
//!   the quad's UVs are the sprite glyph's rect in its sheet (no padding),
//!   its colour the tag's (× the text colour with `tint`, the alpha capped
//!   by both), in a sub-mesh of its sprite asset; its `uv2` as a
//!   character's.
//!
//! Not ported (counted in [`TextMesh::unported`]): texture mappings other
//! than `Character`, vertex gradients, underline and strikethrough.

use crate::font::GlyphRect;
use crate::material::pack_uv;
use crate::mesh::{MeshData, Vertex};

use super::sprite::SpriteAsset;
use super::style;
use super::{TextLayout, TextParams};

/// One font's part of a text: TMP's main mesh (the text's own font) or a
/// sub-mesh (a fallback font).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextMesh {
    pub font: usize,
    /// A sprite asset's sub-mesh (M9u6d1): its index in the set.
    pub sprite: Option<usize>,
    pub mesh: MeshData,
}

/// What the vertices need besides the layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshOptions {
    /// `m_horizontalMapping`, `m_verticalMapping` (0: `Character`).
    pub horizontal_mapping: i32,
    pub vertical_mapping: i32,
    /// `m_isSDFShader`: the text's material has `_WeightNormal`.
    pub sdf_shader: bool,
    /// The text's `|lossyScale.y|` up to its root canvas (the canvas's own
    /// scale, set when the game runs, multiplies in when drawn).
    pub lossy_scale_y: f32,
}

/// A glyph's place in its font's atlas: the rect and the atlas size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasGlyph {
    pub rect: GlyphRect,
    pub atlas_width: i32,
    pub atlas_height: i32,
}

/// The vertices of `layout`. `glyph(font, glyph)` gives a glyph's place
/// in its atlas (`None`: not in the atlas; the character is left out and
/// counted). Returns the meshes and what was not ported or left out.
pub fn text_meshes(
    layout: &TextLayout,
    params: &TextParams,
    options: &MeshOptions,
    glyph: impl FnMut(usize, u32) -> Option<AtlasGlyph>,
) -> (Vec<TextMesh>, Vec<String>) {
    text_meshes_with(layout, params, options, &[], glyph)
}

/// TMP's `Color32.Multiply` (IL): each channel `(byte)(a / 255f × b / 255f
/// × 255f)`, in `f32`, truncated.
fn multiply(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    [0, 1, 2, 3].map(|k| (f32::from(a[k]) / 255.0 * (f32::from(b[k]) / 255.0) * 255.0) as u8)
}

/// [`text_meshes`] with the sprite assets the layout's sprites are from.
pub fn text_meshes_with(
    layout: &TextLayout,
    params: &TextParams,
    options: &MeshOptions,
    sprites: &[SpriteAsset],
    mut glyph: impl FnMut(usize, u32) -> Option<AtlasGlyph>,
) -> (Vec<TextMesh>, Vec<String>) {
    let mut unported = Vec::new();
    if options.horizontal_mapping != 0 || options.vertical_mapping != 0 {
        unported.push(format!(
            "texture mapping {} / {} (drawn as Character)",
            options.horizontal_mapping, options.vertical_mapping
        ));
    }
    let mut out = vec![TextMesh {
        font: params.font,
        sprite: None,
        mesh: MeshData::default(),
    }];
    let mut missing = 0;
    for c in layout.characters.iter().filter(|c| c.visible) {
        // The UV rect: a sprite's own, a glyph's grown by the padding.
        let (uv_rect, sprite) = match c.sprite {
            Some((asset, index)) => {
                let Some((a, g)) = sprites
                    .get(asset)
                    .and_then(|a| Some((a, a.glyph_of(index)?)))
                else {
                    missing += 1;
                    continue;
                };
                let (w, h) = (a.sheet[0] as f32, a.sheet[1] as f32);
                let r = g.rect;
                (
                    [
                        r.x as f32 / w,
                        r.y as f32 / h,
                        (r.x + r.width) as f32 / w,
                        (r.y + r.height) as f32 / h,
                    ],
                    Some(asset),
                )
            }
            None => {
                let Some(g) = glyph(c.font, c.glyph) else {
                    missing += 1;
                    continue;
                };
                let style_padding = if options.sdf_shader {
                    c.style_padding
                } else {
                    0.0
                };
                let (w, h) = (g.atlas_width as f32, g.atlas_height as f32);
                let r = g.rect;
                let reach = c.padding + style_padding;
                (
                    [
                        (r.x as f32 - reach) / w,
                        (r.y as f32 - reach) / h,
                        (r.x as f32 + reach + r.width as f32) / w,
                        (r.y as f32 + reach + r.height as f32) / h,
                    ],
                    None,
                )
            }
        };
        let part = match out
            .iter()
            .position(|m| m.sprite == sprite && (sprite.is_some() || m.font == c.font))
        {
            Some(i) => i,
            None => {
                out.push(TextMesh {
                    font: c.font,
                    sprite,
                    mesh: MeshData::default(),
                });
                out.len() - 1
            }
        };
        let [left, bottom, right, top] = uv_rect;
        let mut scale = c.scale * (1.0 - layout.char_width_adj);
        if !c.alternate && c.style & style::BOLD != 0 {
            scale = -scale;
        }
        scale *= options.lossy_scale_y.abs();
        let mut color = c.color;
        if sprite.is_some() {
            // `SaveSpriteVertexInfo` (vertex colour: the html colour).
            let mut s = if c.tint {
                multiply(c.sprite_color, c.color)
            } else {
                c.sprite_color
            };
            s[3] = if s[3] < params.color[3] {
                s[3].min(c.color[3])
            } else {
                params.color[3]
            };
            color = s;
        } else {
            color[3] = color[3].min(params.color[3]);
        }
        // `Character` mapping: x 0 on the left, 1 on the right; y 0 at
        // the bottom, 1 at the top.
        let corners = [
            (c.bottom_left, [left, bottom], pack_uv(0.0, 0.0)),
            (c.top_left, [left, top], pack_uv(0.0, 1.0)),
            (c.top_right, [right, top], pack_uv(1.0, 1.0)),
            (c.bottom_right, [right, bottom], pack_uv(1.0, 0.0)),
        ];
        let mesh = &mut out[part].mesh;
        let s = mesh.vertices.len() as u32;
        for (pos, uv, packed) in corners {
            mesh.vertices.push(Vertex {
                pos,
                color,
                uv,
                uv1: [packed, scale],
            });
        }
        mesh.indices
            .extend_from_slice(&[s, s + 1, s + 2, s + 2, s + 3, s]);
    }
    if missing > 0 {
        unported.push(format!("{missing} characters without an atlas glyph"));
    }
    (out, unported)
}
