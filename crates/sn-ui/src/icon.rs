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
