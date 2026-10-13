//! uGUI's rects and layout system (M9u4a, `docs/formats/ui.md` § Layout).
//!
//! A [`Tree`] holds what the layout reads: each node's RectTransform, its
//! active flag and its layout components. [`Tree::rect`] gives a node's
//! rectangle as Unity computes it from its anchors; [`layout`] runs what
//! Unity's `LayoutRebuilder` runs when the objects are enabled, and
//! records which values it drove.
//!
//! The rules are written from the IL of `UnityEngine.UI.dll` (2019.4)
//! and of the game's DLL (`uGUI_CircularBar`), read with `sn-inspect code
//! --il`; the code is ours.
//!
//! **Text is not laid out here** (M9u4c): a TMP text's preferred size is
//! `NaN`, and every value computed from it stays `NaN`. The callers count
//! those apart instead of comparing them.

mod rebuild;

pub use rebuild::{Driven, LayoutReport, NoText, TextSizer, layout, layout_with};

/// `[x, y]`.
pub type V2 = [f32; 2];

/// A RectTransform's layout fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectTransform {
    pub anchor_min: V2,
    pub anchor_max: V2,
    pub anchored_position: V2,
    pub size_delta: V2,
    pub pivot: V2,
    /// `localScale`.
    pub scale: [f32; 3],
}

impl Default for RectTransform {
    fn default() -> RectTransform {
        RectTransform {
            anchor_min: [0.5, 0.5],
            anchor_max: [0.5, 0.5],
            anchored_position: [0.0, 0.0],
            size_delta: [100.0, 100.0],
            pivot: [0.5, 0.5],
            scale: [1.0, 1.0, 1.0],
        }
    }
}

/// A rectangle in a node's own space (its pivot at the origin).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn size(&self) -> V2 {
        [self.width, self.height]
    }
}

/// `RectOffset` (integers in Unity).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Padding {
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
}

impl Padding {
    /// `horizontal` (axis 0) or `vertical` (axis 1).
    pub fn along(&self, axis: usize) -> f32 {
        if axis == 0 {
            (self.left + self.right) as f32
        } else {
            (self.top + self.bottom) as f32
        }
    }

    /// Where the children start: `left` (axis 0) or `top` (axis 1).
    pub fn start(&self, axis: usize) -> f32 {
        if axis == 0 {
            self.left as f32
        } else {
            self.top as f32
        }
    }
}

/// `HorizontalLayoutGroup` / `VerticalLayoutGroup`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HvGroup {
    pub vertical: bool,
    pub padding: Padding,
    pub spacing: f32,
    /// `TextAnchor`: 0 upper left … 8 lower right.
    pub alignment: i32,
    /// `childControlWidth`, `childControlHeight`.
    pub control: [bool; 2],
    /// `childForceExpandWidth`, `childForceExpandHeight`.
    pub force_expand: [bool; 2],
    /// `childScaleWidth`, `childScaleHeight`.
    pub use_scale: [bool; 2],
}

/// What an `Image` contributes to layout: its sprite's size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteSize {
    /// `rect` size in pixels.
    pub size: V2,
    /// Left, bottom, right, top.
    pub border: [f32; 4],
    pub pixels_per_unit: f32,
}

/// The layout components, by what they do in the layout.
#[derive(Clone, Debug, PartialEq)]
pub enum Component {
    /// `Image`: a layout element by its sprite. `kind` is `Image.Type`
    /// (0 simple, 1 sliced, 2 tiled, 3 filled).
    Image {
        sprite: Option<SpriteSize>,
        kind: i32,
    },
    /// A TMP text: a layout element whose preferred size is not known
    /// until M9u4c (`NaN`).
    Text,
    LayoutElement {
        ignore: bool,
        min: V2,
        preferred: V2,
        flexible: V2,
        priority: i32,
    },
    Group(HvGroup),
    /// `ContentSizeFitter`: per axis 0 unconstrained, 1 min, 2 preferred.
    SizeFitter {
        fit: [i32; 2],
    },
    /// `AspectRatioFitter`: mode 0 none, 1 width controls height, 2 height
    /// controls width, 3 fit in parent, 4 envelope parent.
    AspectFitter {
        mode: i32,
        ratio: f32,
    },
    /// `uGUI_CircularBar`: sizes its own rect to `size` on both axes.
    CircularBar {
        size: f32,
    },
    /// `ScrollRect`: an element with every value −1 (never chosen), and
    /// a group that drives nothing unless its scrollbars resize the
    /// viewport (logged, not ported).
    ScrollRect,
    /// A component that takes part in layout but is not ported: what it
    /// is to the layout system. Its values are `NaN`.
    Unported {
        name: String,
        element: bool,
        controller: bool,
        group: bool,
        ignorer: bool,
    },
}

impl Component {
    pub fn is_element(&self) -> bool {
        match self {
            Component::Image { .. }
            | Component::Text
            | Component::LayoutElement { .. }
            | Component::Group(_)
            | Component::ScrollRect => true,
            Component::Unported { element, .. } => *element,
            _ => false,
        }
    }

    pub fn is_controller(&self) -> bool {
        match self {
            Component::Group(_)
            | Component::SizeFitter { .. }
            | Component::AspectFitter { .. }
            | Component::CircularBar { .. }
            | Component::ScrollRect => true,
            Component::Unported {
                controller, group, ..
            } => *controller || *group,
            _ => false,
        }
    }

    /// `ILayoutSelfController`: changes its own rect, run before the
    /// other controllers of its object.
    pub fn is_self_controller(&self) -> bool {
        matches!(
            self,
            Component::SizeFitter { .. }
                | Component::AspectFitter { .. }
                | Component::CircularBar { .. }
        )
    }

    pub fn is_group(&self) -> bool {
        match self {
            Component::Group(_) | Component::ScrollRect => true,
            Component::Unported { group, .. } => *group,
            _ => false,
        }
    }

    pub fn is_ignorer(&self) -> bool {
        match self {
            Component::LayoutElement { .. } => true,
            Component::Unported { ignorer, .. } => *ignorer,
            _ => false,
        }
    }
}

/// A component and its `enabled` flag.
#[derive(Clone, Debug, PartialEq)]
pub struct Comp {
    pub kind: Component,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    /// In sibling order.
    pub children: Vec<usize>,
    pub active_self: bool,
    pub rect: RectTransform,
    pub components: Vec<Comp>,
}

/// A UI hierarchy; node 0 is the root.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    pub nodes: Vec<Node>,
    /// `Canvas.referencePixelsPerUnit` (100 without a `CanvasScaler`).
    pub reference_pixels_per_unit: f32,
}

impl Tree {
    pub fn new(nodes: Vec<Node>) -> Tree {
        Tree {
            nodes,
            reference_pixels_per_unit: 100.0,
        }
    }

    /// `activeInHierarchy`.
    pub fn active(&self, mut i: usize) -> bool {
        let mut guard = 0;
        loop {
            let Some(n) = self.nodes.get(i) else {
                return false;
            };
            if !n.active_self {
                return false;
            }
            match n.parent {
                Some(p) if guard < self.nodes.len() => {
                    i = p;
                    guard += 1;
                }
                _ => return true,
            }
        }
    }

    /// The parent's rect size (0 for the root, as Unity has no parent
    /// rect for it).
    pub fn parent_size(&self, i: usize) -> V2 {
        match self.nodes.get(i).and_then(|n| n.parent) {
            Some(p) => self.rect_depth(p, 0).size(),
            None => [0.0, 0.0],
        }
    }

    /// `RectTransform.rect`: size = parent size × (anchor max − anchor
    /// min) + size delta, placed so the pivot is at the origin.
    pub fn rect(&self, i: usize) -> Rect {
        self.rect_depth(i, 0)
    }

    fn rect_depth(&self, i: usize, depth: usize) -> Rect {
        let Some(n) = self.nodes.get(i) else {
            return Rect::default();
        };
        let parent = match n.parent {
            Some(p) if depth < self.nodes.len() => self.rect_depth(p, depth + 1).size(),
            _ => [0.0, 0.0],
        };
        let r = &n.rect;
        let size = [
            parent[0] * (r.anchor_max[0] - r.anchor_min[0]) + r.size_delta[0],
            parent[1] * (r.anchor_max[1] - r.anchor_min[1]) + r.size_delta[1],
        ];
        Rect {
            x: -r.pivot[0] * size[0],
            y: -r.pivot[1] * size[1],
            width: size[0],
            height: size[1],
        }
    }

    /// The node's pivot in its parent's space (`localPosition`'s x and y):
    /// the parent rect's corner + the anchor reference point + the
    /// anchored position.
    pub fn local_position(&self, i: usize) -> V2 {
        let Some(n) = self.nodes.get(i) else {
            return [0.0, 0.0];
        };
        let (prect, psize) = match n.parent {
            Some(p) => {
                let r = self.rect(p);
                ([r.x, r.y], r.size())
            }
            None => ([0.0, 0.0], [0.0, 0.0]),
        };
        let r = &n.rect;
        let mut out = [0.0; 2];
        for a in 0..2 {
            let anchor = r.anchor_min[a] + (r.anchor_max[a] - r.anchor_min[a]) * r.pivot[a];
            out[a] = prect[a] + psize[a] * anchor + r.anchored_position[a];
        }
        out
    }

    /// `RectTransform.SetSizeWithCurrentAnchors`.
    pub fn set_size_with_current_anchors(&mut self, i: usize, axis: usize, size: f32) {
        let parent = self.parent_size(i)[axis];
        if let Some(n) = self.nodes.get_mut(i) {
            let r = &mut n.rect;
            r.size_delta[axis] = size - parent * (r.anchor_max[axis] - r.anchor_min[axis]);
        }
    }

    /// The node's components that are `isActiveAndEnabled`.
    fn enabled(&self, i: usize) -> impl Iterator<Item = &Component> {
        let active = self.active(i);
        self.nodes
            .get(i)
            .into_iter()
            .flat_map(|n| n.components.iter())
            .filter(move |c| active && c.enabled)
            .map(|c| &c.kind)
    }

    /// Hierarchy depth (the root is 0).
    pub fn depth(&self, mut i: usize) -> usize {
        let mut d = 0;
        while let Some(p) = self.nodes.get(i).and_then(|n| n.parent) {
            d += 1;
            i = p;
            if d > self.nodes.len() {
                break;
            }
        }
        d
    }
}

#[cfg(test)]
mod tests;
