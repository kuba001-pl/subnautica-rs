//! `LayoutRebuilder` and the components it runs, as the objects are first
//! enabled: every layout root is rebuilt once, shallowest first (Unity's
//! `CanvasUpdateRegistry` sorts the layout queue by depth), then the
//! aspect ratio fitters update (they do it in `OnEnable`, not in the
//! rebuild: their `SetLayout*` are empty).
//!
//! From the IL (`UnityEngine.UI.dll`):
//! - `MarkLayoutForRebuild`: the root of an object is its highest
//!   ancestor reached through parents with an enabled `ILayoutGroup`; an
//!   object that is its own root needs an enabled `ILayoutController`.
//! - `Rebuild`: `PerformLayoutCalculation` horizontal, `PerformLayoutControl`
//!   horizontal, then both vertical. Calculation recurses into the
//!   children first, if the object has an enabled `ILayoutElement` or any
//!   `ILayoutGroup`; control runs the object's self-controllers, then its
//!   other controllers, then recurses, if it has an enabled controller.
//! - `LayoutUtility.GetLayoutProperty`: over the enabled elements, the
//!   highest `layoutPriority` with a value ≥ 0 wins; at equal priority the
//!   largest value.
//! - `HorizontalOrVerticalLayoutGroup.CalcAlongAxis`, `SetChildrenAlongAxis`,
//!   `GetChildSizes`, `LayoutGroup.GetStartOffset`,
//!   `SetChildAlongAxisWithScale`, `ContentSizeFitter.HandleSelfFittingAlongAxis`,
//!   `AspectRatioFitter.UpdateRect`, `uGUI_CircularBar.HandleLayoutAlongAxis`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{Component, HvGroup, Tree};

/// Which of a node's RectTransform values the layout drove.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Driven {
    /// `anchorMin` and `anchorMax` (set by a parent group or an
    /// enveloping aspect fitter).
    pub anchors: bool,
    pub position: [bool; 2],
    pub size: [bool; 2],
}

impl Driven {
    pub fn any(&self) -> bool {
        self.anchors || self.position.iter().any(|&b| b) || self.size.iter().any(|&b| b)
    }
}

/// What a layout run did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutReport {
    /// The layout roots rebuilt, in order.
    pub roots: Vec<usize>,
    /// Per node.
    pub driven: Vec<Driven>,
    /// Components met that the layout uses but are not ported: name →
    /// count.
    pub unported: BTreeMap<String, usize>,
}

/// Unity's `Mathf.Max`, but `NaN` (an unknown text size) wins.
fn nmax(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a > b {
        a
    } else {
        b
    }
}

/// `Mathf.Clamp` with `NaN` winning.
fn nclamp(v: f32, min: f32, max: f32) -> f32 {
    if v.is_nan() || min.is_nan() || max.is_nan() {
        f32::NAN
    } else if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

fn clamp01(v: f32) -> f32 {
    nclamp(v, 0.0, 1.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// A layout element's property.
#[derive(Clone, Copy)]
enum Prop {
    Min,
    Preferred,
    Flexible,
}

struct Run<'t> {
    tree: &'t mut Tree,
    /// Per group node and axis: total min, preferred, flexible.
    totals: HashMap<usize, [[f32; 3]; 2]>,
    /// Per group node: its layout children (`rectChildren`).
    children: HashMap<usize, Vec<usize>>,
    report: LayoutReport,
}

/// Runs the layout of every root in `tree` and reports what it drove.
pub fn layout(tree: &mut Tree) -> LayoutReport {
    let n = tree.nodes.len();
    let mut run = Run {
        tree,
        totals: HashMap::new(),
        children: HashMap::new(),
        report: LayoutReport {
            driven: vec![Driven::default(); n],
            ..LayoutReport::default()
        },
    };
    let roots = run.roots();
    for &r in &roots {
        run.rebuild(r);
    }
    run.report.roots = roots;
    for i in 0..n {
        run.note_unported(i);
    }
    for i in 0..n {
        let fitters: Vec<(i32, f32)> = run
            .tree
            .enabled(i)
            .filter_map(|c| match c {
                Component::AspectFitter { mode, ratio } => Some((*mode, *ratio)),
                _ => None,
            })
            .collect();
        for (mode, ratio) in fitters {
            run.aspect(i, mode, ratio);
        }
    }
    run.report
}

impl Run<'_> {
    fn note_unported(&mut self, i: usize) {
        let names: Vec<String> = self
            .tree
            .enabled(i)
            .filter_map(|c| match c {
                Component::Unported { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        for name in names {
            *self.report.unported.entry(name).or_default() += 1;
        }
    }

    fn has_enabled(&self, i: usize, f: impl Fn(&Component) -> bool) -> bool {
        self.tree.enabled(i).any(f)
    }

    /// The layout roots, shallowest first (ties in node order).
    fn roots(&self) -> Vec<usize> {
        let mut roots = BTreeSet::new();
        for i in 0..self.tree.nodes.len() {
            if !self.has_enabled(i, Component::is_controller) {
                continue;
            }
            let mut root = i;
            while let Some(p) = self.tree.nodes[root].parent {
                if !self.has_enabled(p, Component::is_group) {
                    break;
                }
                root = p;
            }
            roots.insert((self.tree.depth(root), root));
        }
        roots.into_iter().map(|(_, r)| r).collect()
    }

    fn rebuild(&mut self, root: usize) {
        for axis in 0..2 {
            self.calculate(root, axis, 0);
            self.control(root, axis, 0);
        }
    }

    fn calculate(&mut self, i: usize, axis: usize, depth: usize) {
        if depth > self.tree.nodes.len() {
            return;
        }
        let has_element = self.has_enabled(i, Component::is_element);
        // `GetComponent(typeof(ILayoutGroup))`: enabled or not.
        let has_group = self.tree.nodes[i]
            .components
            .iter()
            .any(|c| c.kind.is_group());
        if !has_element && !has_group {
            return;
        }
        for c in self.tree.nodes[i].children.clone() {
            self.calculate(c, axis, depth + 1);
        }
        let groups: Vec<HvGroup> = self
            .tree
            .enabled(i)
            .filter_map(|c| match c {
                Component::Group(g) => Some(*g),
                _ => None,
            })
            .collect();
        for g in groups {
            if axis == 0 {
                let kids = self.rect_children(i);
                self.children.insert(i, kids);
            }
            self.calc_along_axis(i, &g, axis);
        }
    }

    fn control(&mut self, i: usize, axis: usize, depth: usize) {
        if depth > self.tree.nodes.len() || !self.has_enabled(i, Component::is_controller) {
            return;
        }
        let comps: Vec<Component> = self.tree.enabled(i).cloned().collect();
        for c in comps.iter().filter(|c| c.is_self_controller()) {
            match *c {
                Component::SizeFitter { fit } => {
                    let size = match fit[axis] {
                        1 => Some(self.min_size(i, axis)),
                        2 => Some(self.preferred_size(i, axis)),
                        _ => None,
                    };
                    if let Some(s) = size {
                        self.tree.set_size_with_current_anchors(i, axis, s);
                        self.report.driven[i].size[axis] = true;
                    }
                }
                Component::CircularBar { size } => {
                    self.tree.set_size_with_current_anchors(i, axis, size);
                    self.report.driven[i].size[axis] = true;
                }
                // `SetLayoutHorizontal`/`Vertical` are empty; it works in
                // `OnEnable` (after the rebuilds here).
                _ => {}
            }
        }
        for c in comps
            .iter()
            .filter(|c| c.is_controller() && !c.is_self_controller())
        {
            if let Component::Group(g) = c {
                self.set_children_along_axis(i, g, axis);
            }
        }
        for c in self.tree.nodes[i].children.clone() {
            self.control(c, axis, depth + 1);
        }
    }

    /// `LayoutGroup.rectChildren`: active children, unless every
    /// `ILayoutIgnorer` on them (enabled or not) ignores layout.
    fn rect_children(&self, i: usize) -> Vec<usize> {
        let mut out = Vec::new();
        for &c in &self.tree.nodes[i].children {
            if !self.tree.active(c) {
                continue;
            }
            let ignorers: Vec<bool> = self.tree.nodes[c]
                .components
                .iter()
                .filter(|k| k.kind.is_ignorer())
                .map(|k| match k.kind {
                    Component::LayoutElement { ignore, .. } => ignore,
                    // Not ported: taken as not ignoring.
                    _ => false,
                })
                .collect();
            if ignorers.is_empty() || ignorers.iter().any(|&ig| !ig) {
                out.push(c);
            }
        }
        out
    }

    /// One element's value: (priority, value). `NaN` where unknown.
    fn element_value(&self, i: usize, c: &Component, p: Prop, axis: usize) -> Option<(i32, f32)> {
        let ppu = |s: &super::SpriteSize| s.pixels_per_unit / self.tree.reference_pixels_per_unit;
        Some(match c {
            Component::Image { sprite, kind } => {
                let v = match p {
                    Prop::Min => 0.0,
                    Prop::Flexible => -1.0,
                    Prop::Preferred => match sprite {
                        None => 0.0,
                        Some(s) if *kind == 1 || *kind == 2 => {
                            let b = s.border;
                            let min = if axis == 0 { b[0] + b[2] } else { b[1] + b[3] };
                            min / ppu(s)
                        }
                        Some(s) => s.size[axis] / ppu(s),
                    },
                };
                (0, v)
            }
            Component::Text => (
                0,
                match p {
                    Prop::Min => 0.0,
                    Prop::Flexible => -1.0,
                    Prop::Preferred => f32::NAN,
                },
            ),
            Component::LayoutElement {
                min,
                preferred,
                flexible,
                priority,
                ..
            } => (
                *priority,
                match p {
                    Prop::Min => min[axis],
                    Prop::Preferred => preferred[axis],
                    Prop::Flexible => flexible[axis],
                },
            ),
            Component::Group(_) => {
                let t = self.totals.get(&i).map_or([0.0; 3], |t| t[axis]);
                (
                    0,
                    match p {
                        Prop::Min => t[0],
                        Prop::Preferred => t[1],
                        Prop::Flexible => t[2],
                    },
                )
            }
            Component::ScrollRect => (-1, -1.0),
            Component::Unported { element: true, .. } => (0, f32::NAN),
            _ => return None,
        })
    }

    /// `LayoutUtility.GetLayoutProperty`.
    fn property(&self, i: usize, p: Prop, axis: usize) -> f32 {
        let mut value = 0.0f32;
        let mut max_priority = i32::MIN;
        for c in self.tree.enabled(i) {
            let Some((priority, v)) = self.element_value(i, c, p, axis) else {
                continue;
            };
            if priority < max_priority || v < 0.0 {
                continue;
            }
            if priority > max_priority {
                value = v;
                max_priority = priority;
            } else if v.is_nan() || v > value {
                // An unknown value at the same priority could be the
                // largest: the result is unknown.
                value = v;
            }
        }
        value
    }

    fn min_size(&self, i: usize, axis: usize) -> f32 {
        self.property(i, Prop::Min, axis)
    }

    fn preferred_size(&self, i: usize, axis: usize) -> f32 {
        nmax(
            self.property(i, Prop::Min, axis),
            self.property(i, Prop::Preferred, axis),
        )
    }

    fn flexible_size(&self, i: usize, axis: usize) -> f32 {
        self.property(i, Prop::Flexible, axis)
    }

    /// `GetChildSizes`: (min, preferred, flexible).
    fn child_sizes(&self, c: usize, axis: usize, control: bool, expand: bool) -> [f32; 3] {
        let [min, pref, mut flex] = if control {
            [
                self.min_size(c, axis),
                self.preferred_size(c, axis),
                self.flexible_size(c, axis),
            ]
        } else {
            let s = self.tree.nodes[c].rect.size_delta[axis];
            [s, s, 0.0]
        };
        if expand {
            flex = nmax(flex, 1.0);
        }
        [min, pref, flex]
    }

    fn calc_along_axis(&mut self, i: usize, g: &HvGroup, axis: usize) {
        let padding = g.padding.along(axis);
        let (control, scale, expand) = (g.control[axis], g.use_scale[axis], g.force_expand[axis]);
        let along_other = g.vertical ^ (axis == 1);
        let (mut total_min, mut total_pref, mut total_flex) = (padding, padding, 0.0f32);
        let kids = self.children.get(&i).cloned().unwrap_or_default();
        for &c in &kids {
            let [mut min, mut pref, mut flex] = self.child_sizes(c, axis, control, expand);
            if scale {
                let s = self.tree.nodes[c].rect.scale[axis];
                min *= s;
                pref *= s;
                flex *= s;
            }
            if along_other {
                total_min = nmax(min + padding, total_min);
                total_pref = nmax(pref + padding, total_pref);
                total_flex = nmax(flex, total_flex);
            } else {
                total_min += min + g.spacing;
                total_pref += pref + g.spacing;
                total_flex += flex;
            }
        }
        if !along_other && !kids.is_empty() {
            total_min -= g.spacing;
            total_pref -= g.spacing;
        }
        total_pref = nmax(total_min, total_pref);
        self.totals.entry(i).or_insert([[0.0; 3]; 2])[axis] = [total_min, total_pref, total_flex];
    }

    fn alignment(g: &HvGroup, axis: usize) -> f32 {
        if axis == 0 {
            (g.alignment % 3) as f32 * 0.5
        } else {
            (g.alignment / 3) as f32 * 0.5
        }
    }

    /// `GetStartOffset`.
    fn start_offset(&self, i: usize, g: &HvGroup, axis: usize, required: f32) -> f32 {
        let required = required + g.padding.along(axis);
        let available = self.tree.rect(i).size()[axis];
        let surplus = available - required;
        g.padding.start(axis) + surplus * Self::alignment(g, axis)
    }

    /// `SetChildAlongAxisWithScale`, with a size (the size is driven) or
    /// without.
    fn place(&mut self, c: usize, axis: usize, pos: f32, size: Option<f32>, scale: f32) {
        let d = &mut self.report.driven[c];
        d.anchors = true;
        d.position[axis] = true;
        if size.is_some() {
            d.size[axis] = true;
        }
        let r = &mut self.tree.nodes[c].rect;
        r.anchor_min = [0.0, 1.0];
        r.anchor_max = [0.0, 1.0];
        if let Some(s) = size {
            r.size_delta[axis] = s;
        }
        let s = r.size_delta[axis];
        r.anchored_position[axis] = if axis == 0 {
            pos + s * r.pivot[axis] * scale
        } else {
            -pos - s * (1.0 - r.pivot[axis]) * scale
        };
    }

    fn set_children_along_axis(&mut self, i: usize, g: &HvGroup, axis: usize) {
        let size = self.tree.rect(i).size()[axis];
        let (control, use_scale, expand) =
            (g.control[axis], g.use_scale[axis], g.force_expand[axis]);
        let alignment = Self::alignment(g, axis);
        let along_other = g.vertical ^ (axis == 1);
        let kids = self.children.get(&i).cloned().unwrap_or_default();
        let scale_of = |t: &Tree, c: usize| {
            if use_scale {
                t.nodes[c].rect.scale[axis]
            } else {
                1.0
            }
        };
        if along_other {
            let inner = size - g.padding.along(axis);
            for &c in &kids {
                let [min, pref, flex] = self.child_sizes(c, axis, control, expand);
                let s = scale_of(self.tree, c);
                let required = nclamp(inner, min, if flex > 0.0 { size } else { pref });
                let start = self.start_offset(i, g, axis, required * s);
                if control {
                    self.place(c, axis, start, Some(required), s);
                } else {
                    let offset = (required - self.tree.nodes[c].rect.size_delta[axis]) * alignment;
                    self.place(c, axis, start + offset, None, s);
                }
            }
            return;
        }
        let mut pos = g.padding.start(axis);
        let total = self.totals.get(&i).map_or([0.0; 3], |t| t[axis]);
        let [total_min, total_pref, total_flex] = total;
        let mut flex_multiplier = 0.0;
        let surplus = size - total_pref;
        if surplus > 0.0 {
            if total_flex == 0.0 {
                pos = self.start_offset(i, g, axis, total_pref - g.padding.along(axis));
            } else if total_flex > 0.0 {
                flex_multiplier = surplus / total_flex;
            }
        }
        let mut min_max_lerp = 0.0;
        if total_min != total_pref {
            min_max_lerp = clamp01((size - total_min) / (total_pref - total_min));
        }
        // An unknown total leaves every placement unknown.
        if total.iter().any(|v| v.is_nan()) {
            pos = f32::NAN;
        }
        for &c in &kids {
            let [min, pref, flex] = self.child_sizes(c, axis, control, expand);
            let s = scale_of(self.tree, c);
            let child_size = lerp(min, pref, min_max_lerp) + flex * flex_multiplier;
            if control {
                self.place(c, axis, pos, Some(child_size), s);
            } else {
                let offset = (child_size - self.tree.nodes[c].rect.size_delta[axis]) * alignment;
                self.place(c, axis, pos + offset, None, s);
            }
            pos += child_size * s + g.spacing;
        }
    }

    /// `AspectRatioFitter.UpdateRect`.
    fn aspect(&mut self, i: usize, mode: i32, ratio: f32) {
        let r = self.tree.rect(i);
        match mode {
            1 => {
                self.tree
                    .set_size_with_current_anchors(i, 1, r.width / ratio);
                self.report.driven[i].size[1] = true;
            }
            2 => {
                self.tree
                    .set_size_with_current_anchors(i, 0, r.height * ratio);
                self.report.driven[i].size[0] = true;
            }
            3 | 4 => {
                let parent = self.tree.parent_size(i);
                let rt = &mut self.tree.nodes[i].rect;
                rt.anchor_min = [0.0, 0.0];
                rt.anchor_max = [1.0, 1.0];
                rt.anchored_position = [0.0, 0.0];
                let mut size_delta = [0.0, 0.0];
                // Anchors span the parent: the delta is size − parent.
                if (parent[1] * ratio < parent[0]) ^ (mode == 3) {
                    size_delta[1] = parent[0] / ratio - parent[1];
                } else {
                    size_delta[0] = parent[1] * ratio - parent[0];
                }
                rt.size_delta = size_delta;
                self.report.driven[i] = Driven {
                    anchors: true,
                    position: [true, true],
                    size: [true, true],
                };
            }
            _ => {}
        }
    }
}
