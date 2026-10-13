//! The draw list (M9u4d, `docs/formats/ui.md` § Draw order): every drawn
//! graphic of a UI tree in uGUI's order, its vertices in its canvas's
//! space, its inherited alpha and its clip rectangle.
//!
//! Rules, from the decompiled `UnityEngine.UI.dll` where uGUI's own code
//! decides (the canvas batching itself is native; its order is Unity's
//! documented one):
//! - A root canvas (a `Canvas` with no canvas above it) draws its
//!   graphics depth first in hierarchy order. A nested canvas draws at
//!   its place in that order unless it has `overrideSorting`, in which
//!   case it is a draw group of its own, sorted by its sorting layer and
//!   order. Draw groups sort by (sorting layer's index in the
//!   TagManager, sorting order, hierarchy order).
//! - `CanvasGroup` alpha multiplies down the hierarchy, stopping at a
//!   group with `ignoreParentGroups` (Unity's inherited alpha).
//! - Clipping (`MaskableGraphic.UpdateClipParent`, `RectMask2D.PerformClipping`,
//!   `Clipping.FindCullAndClipWorldRect`, `MaskUtilities.GetRectMasksForClip`):
//!   a maskable graphic is clipped by the intersection of every active,
//!   enabled `RectMask2D` above it (none across a canvas with
//!   `overrideSorting`), each its canvas-space rect (corners 0 and 2)
//!   inset by its padding. A graphic whose rect, in its root canvas's
//!   space, does not overlap the clip rect is culled (`Cull`); an empty
//!   intersection culls everything under it.
//! - A `CanvasRenderer` with `cullTransparentMesh` draws nothing when its
//!   alpha is 0.

use std::collections::HashMap;

use crate::layout::{Rect, Tree, V2};
use crate::mesh::{MeshData, Vertex};

/// A 3×4 affine matrix, row major.
pub type Affine = [[f32; 4]; 3];

pub const IDENTITY: Affine = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

fn mul(a: &Affine, b: &Affine) -> Affine {
    let mut out = [[0.0; 4]; 3];
    for r in 0..3 {
        for c in 0..4 {
            out[r][c] = a[r][0] * b[0][c]
                + a[r][1] * b[1][c]
                + a[r][2] * b[2][c]
                + if c == 3 { a[r][3] } else { 0.0 };
        }
    }
    out
}

pub fn apply(m: &Affine, p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
        m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
        m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
    ]
}

/// Translation · rotation (unit quaternion x, y, z, w) · scale.
pub fn trs(t: [f32; 3], q: [f32; 4], s: [f32; 3]) -> Affine {
    let [x, y, z, w] = q;
    let r = [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ];
    let mut m = [[0.0; 4]; 3];
    for row in 0..3 {
        for col in 0..3 {
            m[row][col] = r[row][col] * s[col];
        }
        m[row][3] = t[row];
    }
    m
}

/// A canvas of the tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasInfo {
    pub enabled: bool,
    pub override_sorting: bool,
    /// The sorting layer's index in the TagManager's list.
    pub sorting_layer: i32,
    pub sorting_order: i32,
}

/// A `CanvasGroup`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupInfo {
    pub alpha: f32,
    pub ignore_parent_groups: bool,
    pub enabled: bool,
}

/// A `RectMask2D`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskInfo {
    pub enabled: bool,
    /// Left, bottom, right, top.
    pub padding: [f32; 4],
    pub softness: [i32; 2],
}

/// A graphic ready to draw: its mesh in its node's space.
#[derive(Clone, Debug, PartialEq)]
pub struct Graphic {
    pub node: usize,
    pub mesh: MeshData,
    /// Which texture (the caller's key; 0 the white texture).
    pub texture: u64,
    pub maskable: bool,
    pub cull_transparent: bool,
    /// What it is, for the logs.
    pub kind: String,
}

/// What the draw list reads besides the laid-out tree.
#[derive(Default)]
pub struct DrawInput {
    pub canvases: HashMap<usize, CanvasInfo>,
    pub groups: HashMap<usize, GroupInfo>,
    pub masks: HashMap<usize, MaskInfo>,
    pub graphics: Vec<Graphic>,
}

/// One drawn graphic.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawItem {
    pub node: usize,
    /// The draw group's canvas node (a root canvas or an
    /// override-sorting one).
    pub canvas: usize,
    /// Vertices in the root canvas's space, alpha already multiplied in.
    pub vertices: Vec<Vertex>,
    pub z: Vec<f32>,
    pub indices: Vec<u32>,
    pub texture: u64,
    pub alpha: f32,
    /// In the root canvas's space.
    pub clip: Option<Rect>,
    pub softness: [i32; 2],
    pub kind: String,
}

/// The draw list and what was left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    pub items: Vec<DrawItem>,
    /// Graphics culled by a clip rect.
    pub culled: usize,
    /// Graphics with `cullTransparentMesh` and alpha 0.
    pub transparent: usize,
    /// Graphics not under any canvas.
    pub without_canvas: usize,
}

impl DrawList {
    /// FNV-1a over every item's numbers (bit patterns): the same input
    /// gives the same hash.
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |v: u32| {
            for b in v.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x100_0000_01b3);
            }
        };
        for it in &self.items {
            eat(it.node as u32);
            eat(it.canvas as u32);
            eat(it.texture as u32);
            eat(it.alpha.to_bits());
            for v in &it.vertices {
                eat(v.pos[0].to_bits());
                eat(v.pos[1].to_bits());
                eat(v.uv[0].to_bits());
                eat(v.uv[1].to_bits());
                eat(u32::from_le_bytes(v.color));
            }
            for &i in &it.indices {
                eat(i);
            }
            if let Some(c) = it.clip {
                for v in [c.x, c.y, c.width, c.height] {
                    eat(v.to_bits());
                }
            }
        }
        h
    }
}

fn ancestors(tree: &Tree, i: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut k = tree.nodes.get(i).and_then(|n| n.parent);
    while let Some(p) = k {
        if out.len() > tree.nodes.len() {
            break;
        }
        out.push(p);
        k = tree.nodes[p].parent;
    }
    out
}

/// The node's local transform (its pivot's place in its parent).
fn local(tree: &Tree, i: usize) -> Affine {
    let r = &tree.nodes[i].rect;
    let p = tree.local_position(i);
    trs([p[0], p[1], r.z], r.rotation, r.scale)
}

/// From the node's space to `to`'s (an ancestor).
fn to_ancestor(tree: &Tree, i: usize, to: usize) -> Affine {
    let mut m = IDENTITY;
    let mut k = i;
    let mut guard = 0;
    while k != to && guard <= tree.nodes.len() {
        m = mul(&local(tree, k), &m);
        match tree.nodes[k].parent {
            Some(p) => k = p,
            None => break,
        }
        guard += 1;
    }
    m
}

/// A rect's corners 0 and 2 mapped by `m`, as Unity's canvas rects.
fn corner_rect(rect: Rect, m: &Affine) -> Rect {
    let a = apply(m, [rect.x, rect.y, 0.0]);
    let b = apply(m, [rect.x + rect.width, rect.y + rect.height, 0.0]);
    Rect {
        x: a[0],
        y: a[1],
        width: b[0] - a[0],
        height: b[1] - a[1],
    }
}

/// `Rect.Overlaps(other, allowInverse: true)`.
fn overlaps(a: Rect, b: Rect) -> bool {
    let norm = |r: Rect| {
        let (x0, x1) = if r.width < 0.0 {
            (r.x + r.width, r.x)
        } else {
            (r.x, r.x + r.width)
        };
        let (y0, y1) = if r.height < 0.0 {
            (r.y + r.height, r.y)
        } else {
            (r.y, r.y + r.height)
        };
        (x0, y0, x1, y1)
    };
    let (ax0, ay0, ax1, ay1) = norm(a);
    let (bx0, by0, bx1, by1) = norm(b);
    bx1 > ax0 && bx0 < ax1 && by1 > ay0 && by0 < ay1
}

/// Builds the draw list of `tree` (laid out).
pub fn draw_list(tree: &Tree, input: &DrawInput) -> DrawList {
    let mut out = DrawList::default();
    let n = tree.nodes.len();
    let canvas_of = |i: usize| -> Option<usize> {
        std::iter::once(i)
            .chain(ancestors(tree, i))
            .find(|k| input.canvases.contains_key(k))
    };
    // The root canvas of a node: its farthest canvas ancestor.
    let root_canvas_of = |i: usize| -> Option<usize> {
        let chain: Vec<usize> = std::iter::once(i).chain(ancestors(tree, i)).collect();
        chain
            .into_iter()
            .rev()
            .find(|k| input.canvases.contains_key(k))
    };
    // The draw group: the nearest canvas with override sorting, or the
    // root canvas.
    let group_of = |i: usize| -> Option<usize> {
        let chain: Vec<usize> = std::iter::once(i).chain(ancestors(tree, i)).collect();
        chain
            .iter()
            .copied()
            .find(|k| input.canvases.get(k).is_some_and(|c| c.override_sorting))
            .or_else(|| root_canvas_of(i))
    };
    let alpha_of = |i: usize| -> f32 {
        let mut a = 1.0;
        for k in std::iter::once(i).chain(ancestors(tree, i)) {
            if let Some(g) = input.groups.get(&k).filter(|g| g.enabled && tree.active(k)) {
                a *= g.alpha;
                if g.ignore_parent_groups {
                    break;
                }
            }
        }
        a
    };
    // Hierarchy order: depth first, children in order.
    let mut order = vec![usize::MAX; n];
    let mut next = 0;
    let mut stack: Vec<usize> = (0..n)
        .filter(|&i| tree.nodes[i].parent.is_none())
        .rev()
        .collect();
    while let Some(i) = stack.pop() {
        if order[i] != usize::MAX {
            continue;
        }
        order[i] = next;
        next += 1;
        for &c in tree.nodes[i].children.iter().rev() {
            stack.push(c);
        }
    }
    let mut items: Vec<(i32, i32, usize, DrawItem)> = Vec::new();
    for g in &input.graphics {
        let i = g.node;
        if i >= n || !tree.active(i) {
            continue;
        }
        let (Some(group), Some(root)) = (group_of(i), root_canvas_of(i)) else {
            out.without_canvas += 1;
            continue;
        };
        if !input.canvases.get(&root).is_some_and(|c| c.enabled) {
            continue;
        }
        let alpha = alpha_of(i);
        if g.cull_transparent && alpha <= 0.0 {
            out.transparent += 1;
            continue;
        }
        // Clipping: the masks above, not across an override canvas.
        let mut clip: Option<Rect> = None;
        let mut softness = [0, 0];
        if g.maskable {
            let mut masks = Vec::new();
            let mut crossed_override = false;
            for k in std::iter::once(i).chain(ancestors(tree, i)) {
                if crossed_override {
                    break;
                }
                if let Some(m) = input.masks.get(&k).filter(|m| m.enabled && tree.active(k)) {
                    masks.push((k, *m));
                }
                if input.canvases.get(&k).is_some_and(|c| c.override_sorting) {
                    crossed_override = true;
                }
            }
            if !masks.is_empty() {
                let mut r = [f32::MIN, f32::MIN, f32::MAX, f32::MAX];
                for (k, m) in &masks {
                    let canvas = canvas_of(*k).unwrap_or(root);
                    let cr = corner_rect(tree.rect(*k), &to_ancestor(tree, *k, canvas));
                    r[0] = r[0].max(cr.x + m.padding[0]);
                    r[1] = r[1].max(cr.y + m.padding[1]);
                    r[2] = r[2].min(cr.x + cr.width - m.padding[2]);
                    r[3] = r[3].min(cr.y + cr.height - m.padding[3]);
                }
                if let Some((_, m)) = masks.first() {
                    softness = m.softness;
                }
                let valid = r[2] > r[0] && r[3] > r[1];
                let rect = Rect {
                    x: r[0],
                    y: r[1],
                    width: r[2] - r[0],
                    height: r[3] - r[1],
                };
                let own = corner_rect(tree.rect(i), &to_ancestor(tree, i, root));
                if !valid || !overlaps(rect, own) {
                    out.culled += 1;
                    continue;
                }
                clip = Some(rect);
            }
        }
        let m = to_ancestor(tree, i, root);
        let mut vertices = Vec::with_capacity(g.mesh.vertices.len());
        let mut z = Vec::with_capacity(g.mesh.vertices.len());
        for v in &g.mesh.vertices {
            let p = apply(&m, [v.pos[0], v.pos[1], 0.0]);
            let mut color = v.color;
            color[3] = (f32::from(color[3]) * alpha).round().clamp(0.0, 255.0) as u8;
            vertices.push(Vertex {
                pos: [p[0], p[1]] as V2,
                color,
                uv: v.uv,
            });
            z.push(p[2]);
        }
        let info = input.canvases.get(&group).copied().unwrap_or(CanvasInfo {
            enabled: true,
            override_sorting: false,
            sorting_layer: 0,
            sorting_order: 0,
        });
        items.push((
            info.sorting_layer,
            info.sorting_order,
            order[i],
            DrawItem {
                node: i,
                canvas: group,
                vertices,
                z,
                indices: g.mesh.indices.clone(),
                texture: g.texture,
                alpha,
                clip,
                softness,
                kind: g.kind.clone(),
            },
        ));
    }
    // Stable: equal keys keep hierarchy order.
    items.sort_by_key(|(layer, ord, h, _)| (*layer, *ord, *h));
    out.items = items.into_iter().map(|(_, _, _, it)| it).collect();
    out
}

#[cfg(test)]
mod tests;
