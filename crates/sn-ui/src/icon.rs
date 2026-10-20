//! `uGUI_Icon` and the parts of `uGUI_ItemIcon` the icon notifier uses
//! (M9u7c1; `docs/formats/ui.md` § Picked-up icons), ported from the
//! decompiled scripts (read only; the code is ours).
//!
//! - [`icon_mesh`]: `uGUI_Icon.OnPopulateMesh`: the sprite's own mesh
//!   (`GeneratePackedMesh`) scaled to the rect, or one quad without a
//!   sprite (`GenerateQuadMesh`). Its second UV is the vertex position.
//! - [`fit`]: `RectTransformExtensions.Fit`, how `SetForegroundSize` sizes
//!   the foreground to its sprite.
//! - [`sprite_uv`]: `Sprite.uv` from the render data's `uvTransform`.
//!
//! `uGUI_ItemIcon` itself is a `Graphic` whose `OnPopulateMesh` clears the
//! mesh: it draws nothing; its children (the foreground) do.

use crate::layout::{Rect, V2};
use crate::mesh::{MeshData, Vertex, color32};

/// The shader of `uGUI_ItemIcon.iconMaterial`
/// (`ShaderManager.preloadedShaders.UIIcon`).
pub const SHADER: &str = "UI/Icon";

/// `uGUI_ItemIcon.iconSize`'s use: `SetSize(w, h)` sizes the icon and its
/// foreground; a new `RectTransform`'s `sizeDelta` before that.
pub const NEW_RECT_SIZE: V2 = [100.0, 100.0];

/// A sprite as `uGUI_Icon` draws it: `SpriteManager.GetSpriteCache`'s
/// `vertices`, `uv` and `triangles`, with `sprite.rect`'s size and
/// `pixelsPerUnit`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IconSprite {
    /// `sprite.rect.size`, pixels.
    pub size: V2,
    pub pixels_per_unit: f32,
    /// Units, about the sprite's pivot.
    pub vertices: Vec<V2>,
    pub uv: Vec<V2>,
    pub triangles: Vec<u16>,
}

/// `Sprite.uv` of a vertex at `p` (units): the render data's
/// `uvTransform` (pixels per unit and the pivot's place in the page, per
/// axis: x scale, x offset, y scale, y offset) over the page's size
/// (*hypothesis*: the stored UV stream of the game's atlas sprites is
/// zeros, so the engine derives it; this rule puts every vertex of a
/// packed sprite in its packed rect, `ui --icon-mesh`).
pub fn sprite_uv(p: V2, uv_transform: [f32; 4], page: V2) -> V2 {
    [
        (p[0] * uv_transform[0] + uv_transform[1]) / page[0],
        (p[1] * uv_transform[2] + uv_transform[3]) / page[1],
    ]
}

/// `uGUI_Icon.OnPopulateMesh` in a rect `rect` (its local space, the
/// pivot at the origin; `GetPixelAdjustedRect` is the rect on a canvas
/// that is not pixel perfect) with `Graphic.color` `color`.
pub fn icon_mesh(rect: Rect, sprite: Option<&IconSprite>, color: [f32; 4]) -> MeshData {
    let c = color32(color);
    let mut m = MeshData::default();
    let Some(s) = sprite else {
        // `GenerateQuadMesh`: UVs 0…1, the second UV the corner.
        let (min, max) = (
            [rect.x, rect.y],
            [rect.x + rect.width, rect.y + rect.height],
        );
        let corners = [min, [min[0], max[1]], max, [max[0], min[1]]];
        let uv = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        for k in 0..4 {
            m.vertices.push(Vertex {
                pos: corners[k],
                color: c,
                uv: uv[k],
                uv1: corners[k],
            });
        }
        m.indices.extend_from_slice(&[0, 1, 2, 2, 3, 0]);
        return m;
    };
    // `GeneratePackedMesh`: each vertex × (rect ÷ sprite size) × ppu,
    // the rect's position not added (the sprite's pivot is the rect's
    // origin).
    let b = [
        rect.width / s.size[0] * s.pixels_per_unit,
        rect.height / s.size[1] * s.pixels_per_unit,
    ];
    for (k, v) in s.vertices.iter().enumerate() {
        let a = [v[0] * b[0], v[1] * b[1]];
        m.vertices.push(Vertex {
            pos: a,
            color: c,
            uv: s.uv.get(k).copied().unwrap_or([0.0, 0.0]),
            uv1: a,
        });
    }
    m.indices.extend(s.triangles.iter().map(|&i| u32::from(i)));
    m
}

/// `RectTransformExtensions.Fit(desired, default, keepAspect)`: a
/// negative desired size takes the default's (or keeps its aspect from
/// the other axis); keeping the aspect scales the default to fit inside.
pub fn fit(desired: V2, default: V2, keep_aspect: bool) -> V2 {
    let aspect = default[0] / default[1];
    match (desired[0] < 0.0, desired[1] < 0.0) {
        (true, true) => default,
        (true, false) => [aspect * desired[1], desired[1]],
        (false, true) => [desired[0], desired[0] / aspect],
        (false, false) if keep_aspect => {
            let k = (desired[0] / default[0]).min(desired[1] / default[1]);
            [default[0] * k, default[1] * k]
        }
        _ => desired,
    }
}

/// `UpdateFillProperties`' `_FillRect`: the larger of the background's
/// and the foreground's sizes, centred.
pub fn fill_rect(background: V2, foreground: V2) -> [f32; 4] {
    let v = [
        background[0].max(foreground[0]),
        background[1].max(foreground[1]),
    ];
    [-0.5 * v[0], -0.5 * v[1], v[0], v[1]]
}

/// `UI/Icon`'s `SLICE_9_GRID` fragment program's sample coordinate (M9u8b2b,
/// from its D3D11 disassembly; `game_ui.wgsl` mirrors it): the quad's UV
/// mapped onto a strip texture `tex_height` texels tall wrapped round a
/// rounded border. The border is R = min(½·min(size), `_Radius`) units, b =
/// R ÷ `_Size` of the UV. Along an edge s runs along it (0…1 between the
/// corners) and t goes from 0 at the border's inner line to 1 at the edge.
/// In a corner s = ½ and t is the distance from the corner's inner point
/// (in border widths). Inside, (½, 0). Then t is rescaled so the strip's
/// height covers the outer `tex_height` units: t' = (t − k) ÷ (1 − k), k =
/// (R − h) ÷ R.
pub fn slice9_uv(uv: V2, radius: f32, size: V2, tex_height: f32) -> V2 {
    let r = (size[1].min(size[0]) * 0.5).min(radius);
    let b = [r / size[0], r / size[1]];
    let (left, not_right) = (uv[0] <= b[0], uv[0] <= 1.0 - b[0]);
    let mid_u = (uv[0] - b[0]) / (1.0 - 2.0 * b[0]);
    let t_left = 1.0 - uv[0] / b[0];
    let t_right = (uv[0] - (1.0 - b[0])) / b[0];
    let len = |a: f32, c: f32| (a * a + c * c).sqrt();
    let st = if uv[1] <= b[1] {
        let t_bottom = 1.0 - uv[1] / b[1];
        if left {
            [0.5, len(t_left, t_bottom)]
        } else if not_right {
            [mid_u, t_bottom]
        } else {
            [0.5, len(t_right, t_bottom)]
        }
    } else if uv[1] <= 1.0 - b[1] {
        let mid_v = (uv[1] - b[1]) / (1.0 - 2.0 * b[1]);
        if left {
            [mid_v, t_left]
        } else if not_right {
            [0.5, 0.0]
        } else {
            [mid_v, t_right]
        }
    } else {
        let t_top = (uv[1] - (1.0 - b[1])) / b[1];
        if left {
            [0.5, len(t_top, t_left)]
        } else if not_right {
            [mid_u, t_top]
        } else {
            [0.5, len(t_top, t_right)]
        }
    };
    let k = (r - tex_height) / r;
    [st[0], (st[1] - k) / (1.0 - k)]
}

/// `UI/Circle`'s fragment program's sample coordinate (M9u8b2b, from its
/// D3D11 disassembly; `game_ui.wgsl` mirrors it): p = 2·UV − 1, folded to
/// the right half (q = −p and + ½ turn on the left); u = (atan2(q.y, q.x)
/// ÷ 2π + that half turn) × `_TileX`; v = saturate((|p| − k) ÷ (1 − k)) ×
/// `_TileY`, k = (`_Size` − 2·h) ÷ `_Size` (h the texture's height): the
/// texture covers the outer 2h units of the radius.
pub fn circle_uv(uv: V2, tile: V2, size: f32, tex_height: f32) -> V2 {
    let p = [2.0 * uv[0] - 1.0, 2.0 * uv[1] - 1.0];
    let (q, half) = if p[0] < 0.0 {
        ([-p[0], -p[1]], 0.5)
    } else {
        (p, 0.0)
    };
    let u = (q[1].atan2(q[0]) * 0.159_155 + half) * tile[0];
    let k = (size - 2.0 * tex_height) / size;
    let r = (p[0] * p[0] + p[1] * p[1]).sqrt();
    let v = ((r - k) / (1.0 - k)).clamp(0.0, 1.0) * tile[1];
    [u, v]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: f32, h: f32) -> Rect {
        Rect {
            x: -w / 2.0,
            y: -h / 2.0,
            width: w,
            height: h,
        }
    }

    #[test]
    fn a_packed_sprite_is_scaled_to_the_rect() {
        // A 128-pixel sprite at 100 pixels per unit drawn 64 wide: × 50.
        let s = IconSprite {
            size: [128.0, 128.0],
            pixels_per_unit: 100.0,
            vertices: vec![[0.4, 0.12], [0.3, -0.39], [-0.41, 0.22]],
            uv: vec![[0.1, 0.2], [0.3, 0.4], [0.5, 0.6]],
            triangles: vec![2, 1, 0],
        };
        let m = icon_mesh(rect(64.0, 64.0), Some(&s), [1.0, 1.0, 1.0, 0.5]);
        assert_eq!(m.vertices.len(), 3);
        assert_eq!(m.indices, [2, 1, 0]);
        let v = m.vertices[0];
        assert!((v.pos[0] - 20.0).abs() < 1e-5 && (v.pos[1] - 6.0).abs() < 1e-5);
        assert_eq!(v.uv1, v.pos);
        assert_eq!(v.uv, [0.1, 0.2]);
        assert_eq!(v.color, [255, 255, 255, 128]);
        // Non-square: each axis on its own.
        let m = icon_mesh(rect(32.0, 64.0), Some(&s), [1.0; 4]);
        assert!((m.vertices[0].pos[0] - 10.0).abs() < 1e-5);
        assert!((m.vertices[0].pos[1] - 6.0).abs() < 1e-5);
    }

    #[test]
    fn without_a_sprite_it_is_one_quad() {
        let m = icon_mesh(rect(10.0, 20.0), None, [1.0; 4]);
        let pos: Vec<V2> = m.vertices.iter().map(|v| v.pos).collect();
        assert_eq!(
            pos,
            [[-5.0, -10.0], [-5.0, 10.0], [5.0, 10.0], [5.0, -10.0]]
        );
        assert_eq!(m.vertices[2].uv, [1.0, 1.0]);
        assert_eq!(m.vertices[1].uv1, [-5.0, 10.0]);
        assert_eq!(m.indices, [0, 1, 2, 2, 3, 0]);
    }

    #[test]
    fn fit_keeps_the_aspect_or_takes_the_default() {
        assert_eq!(fit([-1.0, -1.0], [128.0, 96.0], true), [128.0, 96.0]);
        assert_eq!(fit([64.0, 64.0], [128.0, 128.0], true), [64.0, 64.0]);
        assert_eq!(fit([64.0, 64.0], [128.0, 64.0], true), [64.0, 32.0]);
        assert_eq!(fit([64.0, 64.0], [128.0, 64.0], false), [64.0, 64.0]);
        assert_eq!(fit([-1.0, 32.0], [128.0, 64.0], true), [64.0, 32.0]);
        assert_eq!(fit([32.0, -1.0], [128.0, 64.0], true), [32.0, 16.0]);
    }

    fn near(a: V2, b: V2) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    #[test]
    fn slice9_maps_the_border() {
        // A 1 × 1 icon (66 units, radius 33): the border is the whole half;
        // the lower left quarter's point is a corner at √½.
        let h = 33.0;
        let s = slice9_uv([0.25, 0.25], 33.0, [66.0, 66.0], h);
        assert!(near(s, [0.5, 0.5f32.sqrt()]), "{s:?}");
        // 2 × 2 (137 units): b = 33 / 137 = 0.240876.
        let sz = [137.0, 137.0];
        let b = 33.0f32 / 137.0;
        // Bottom edge, middle: along 0.5, across 1 − 0.1 / b.
        assert!(near(
            slice9_uv([0.5, 0.1], 33.0, sz, h),
            [0.5, 1.0 - 0.1 / b]
        ));
        // Inside.
        assert!(near(slice9_uv([0.5, 0.5], 33.0, sz, h), [0.5, 0.0]));
        // Right edge: along (0.5 − b) / (1 − 2b) = 0.5, across (0.9 − (1 − b)) / b.
        assert!(near(
            slice9_uv([0.9, 0.5], 33.0, sz, h),
            [0.5, (0.9 - (1.0 - b)) / b]
        ));
        // Top right corner: the distance in border widths.
        let t = (0.95 - (1.0 - b)) / b;
        assert!(near(
            slice9_uv([0.95, 0.95], 33.0, sz, h),
            [0.5, (2.0 * t * t).sqrt()]
        ));
        // A taller strip (68): k = (33 − 68) / 33; the inner line maps to
        // −k / (1 − k) = 35 / 68.
        let s = slice9_uv([0.5, 0.5], 33.0, sz, 68.0);
        assert!(near(s, [0.5, 35.0 / 68.0]), "{s:?}");
        // The radius is capped at half the smaller side.
        let s = slice9_uv([0.5, 0.05], 100.0, [40.0, 20.0], 10.0);
        // R = 10: b = (0.25, 0.5); t = 1 − 0.05 / 0.5 = 0.9; k = 0.
        assert!(near(s, [0.5, 0.9]), "{s:?}");
    }

    #[test]
    fn circle_maps_angle_and_radius() {
        // Right of the centre, on the rim: angle 0, |p| = 1 → v 1.
        assert!(near(
            circle_uv([1.0, 0.5], [1.0, 1.0], 100.0, 10.0),
            [0.0, 1.0]
        ));
        // Straight up, halfway out: u ¼; k = 0.8, |p| = 0.5 → v 0.
        assert!(near(
            circle_uv([0.5, 0.75], [1.0, 1.0], 100.0, 10.0),
            [0.25, 0.0]
        ));
        // Left of the centre: folded, + ½ turn; |p| 0.9 → (0.9 − 0.8) / 0.2.
        let c = circle_uv([0.05, 0.5], [2.0, 3.0], 100.0, 10.0);
        assert!(near(c, [1.0, 0.5 * 3.0]), "{c:?}");
        // Straight down: −¼ turn.
        assert!(near(
            circle_uv([0.5, 0.0], [1.0, 1.0], 100.0, 10.0),
            [-0.25, 1.0]
        ));
    }

    #[test]
    fn a_vertex_uv_comes_from_the_uv_transform() {
        // The game's titanium: (100, 245, 100, 211) in a 2048 page; its top
        // vertex (0.21, 0.5) is at pixel (266, 261).
        let uv = sprite_uv([0.21, 0.5], [100.0, 245.0, 100.0, 211.0], [2048.0, 2048.0]);
        assert!((uv[0] * 2048.0 - 266.0).abs() < 1e-3);
        assert!((uv[1] * 2048.0 - 261.0).abs() < 1e-3);
        assert_eq!(
            fill_rect([-1.0, -1.0], [64.0, 48.0]),
            [-32.0, -24.0, 64.0, 48.0]
        );
    }
}
