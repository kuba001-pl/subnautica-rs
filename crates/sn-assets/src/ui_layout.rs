//! A UI hierarchy as `sn-ui`'s layout reads it (M9u4a): each node's
//! RectTransform and its layout components, their fields read by the
//! layouts from the game's assemblies (M9u1).

use sn_ui::layout::{Comp, Component, HvGroup, Node, Padding, RectTransform, SpriteSize, Tree};
use sn_unity::{Fields, MonoBehaviourHeader, Sprite, Value};

use crate::{Assets, Layouts, ObjectRef, Result, UiComponent, UiTree};

const RECT_TRANSFORM: i32 = 224;
const MONO_BEHAVIOUR: i32 = 114;
const SPRITE: i32 = 213;

/// Scripts that take part in layout but are not ported, and what they
/// are to the layout system (element, controller, group, ignorer); from
/// the interfaces they declare (`sn-inspect code --implements`) or
/// inherit (`LayoutGroup`, `Text`, `InputField`).
const UNPORTED: [(&str, [bool; 4]); 11] = [
    ("FlexibleGridLayout", [true, true, true, false]),
    ("GridLayoutGroup", [true, true, true, false]),
    ("Text", [true, false, false, false]),
    ("InputField", [true, false, false, false]),
    ("TMP_InputField", [true, false, false, false]),
    ("AspectRatioLayoutElement", [true, false, false, false]),
    ("uGUI_BlueprintEntry", [true, false, false, false]),
    ("uGUI_IconGrid", [true, false, false, false]),
    ("uGUI_TooltipIcon", [true, false, false, false]),
    ("uGUI_BlueprintProgress", [false, false, false, true]),
    ("uGUI_NotificationLabel", [false, false, false, true]),
];

fn f32_of(f: &Fields, name: &str) -> Result<f32> {
    f.get(name)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .ok_or_else(|| format!("no number {name}"))
}

fn i32_of(f: &Fields, name: &str) -> Result<i32> {
    f.get(name)
        .and_then(Value::as_i64)
        .map(|v| v as i32)
        .ok_or_else(|| format!("no integer {name}"))
}

fn bool_of(f: &Fields, name: &str) -> Result<bool> {
    f.get(name)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("no bool {name}"))
}

/// A UI hierarchy ready for layout, and what could not be read.
pub struct UiLayout {
    pub tree: Tree,
    /// Problems met (sprites that do not resolve, …), one line each.
    pub notes: Vec<String>,
}

impl Assets<'_> {
    fn script_fields(&self, layouts: &Layouts, c: &UiComponent) -> Result<(bool, Fields)> {
        let script = self
            .mono_script(&c.object)
            .ok_or("MonoBehaviour without its script")?;
        let layout = layouts.script(&script.assembly, &script.namespace, &script.class_name)?;
        let (_, d) = c.object.data()?;
        let (MonoBehaviourHeader { enabled, .. }, fields) =
            sn_unity::read_script(d, c.object.file.file().big_endian, &layout)
                .map_err(|e| format!("{}: {e}", script.class_name))?;
        Ok((enabled, fields))
    }

    fn sprite_size(&self, from: &ObjectRef, f: &Fields) -> Result<Option<SpriteSize>> {
        let Some(p) = f.get("m_Sprite").and_then(Value::as_pptr) else {
            return Ok(None);
        };
        let Some(o) = self.resolve(&from.file, p)? else {
            return Ok(None);
        };
        let (info, d) = o.data()?;
        if info.class_id != SPRITE {
            return Err(format!("m_Sprite is class {}", info.class_id));
        }
        let s = Sprite::parse(d, o.file.file().big_endian).map_err(|e| format!("Sprite: {e}"))?;
        Ok(Some(SpriteSize {
            size: [s.rect[2], s.rect[3]],
            border: s.border,
            pixels_per_unit: s.pixels_to_units,
        }))
    }

    fn layout_component(
        &self,
        layouts: &Layouts,
        c: &UiComponent,
        notes: &mut Vec<String>,
        path: &str,
    ) -> Result<Option<Comp>> {
        let Some(script) = c.script.as_deref() else {
            return Ok(None);
        };
        let interesting = matches!(
            script,
            "Image"
                | "TextMeshProUGUI"
                | "LayoutElement"
                | "HorizontalLayoutGroup"
                | "VerticalLayoutGroup"
                | "ContentSizeFitter"
                | "AspectRatioFitter"
                | "uGUI_CircularBar"
                | "ScrollRect"
        ) || UNPORTED.iter().any(|(n, _)| *n == script);
        if !interesting {
            return Ok(None);
        }
        let (enabled, f) = self.script_fields(layouts, c)?;
        let kind = match script {
            "Image" => {
                let sprite = match self.sprite_size(&c.object, &f) {
                    Ok(s) => s,
                    Err(e) => {
                        notes.push(format!("{path}: Image sprite: {e}"));
                        None
                    }
                };
                Component::Image {
                    sprite,
                    kind: i32_of(&f, "m_Type")?,
                }
            }
            "TextMeshProUGUI" => Component::Text,
            "LayoutElement" => Component::LayoutElement {
                ignore: bool_of(&f, "m_IgnoreLayout")?,
                min: [f32_of(&f, "m_MinWidth")?, f32_of(&f, "m_MinHeight")?],
                preferred: [
                    f32_of(&f, "m_PreferredWidth")?,
                    f32_of(&f, "m_PreferredHeight")?,
                ],
                flexible: [
                    f32_of(&f, "m_FlexibleWidth")?,
                    f32_of(&f, "m_FlexibleHeight")?,
                ],
                priority: i32_of(&f, "m_LayoutPriority")?,
            },
            "HorizontalLayoutGroup" | "VerticalLayoutGroup" => {
                let p = f
                    .get("m_Padding")
                    .and_then(Value::as_struct)
                    .ok_or("no m_Padding")?;
                Component::Group(HvGroup {
                    vertical: script == "VerticalLayoutGroup",
                    padding: Padding {
                        left: i32_of(p, "m_Left")?,
                        right: i32_of(p, "m_Right")?,
                        top: i32_of(p, "m_Top")?,
                        bottom: i32_of(p, "m_Bottom")?,
                    },
                    spacing: f32_of(&f, "m_Spacing")?,
                    alignment: i32_of(&f, "m_ChildAlignment")?,
                    control: [
                        bool_of(&f, "m_ChildControlWidth")?,
                        bool_of(&f, "m_ChildControlHeight")?,
                    ],
                    force_expand: [
                        bool_of(&f, "m_ChildForceExpandWidth")?,
                        bool_of(&f, "m_ChildForceExpandHeight")?,
                    ],
                    use_scale: [
                        bool_of(&f, "m_ChildScaleWidth")?,
                        bool_of(&f, "m_ChildScaleHeight")?,
                    ],
                })
            }
            "ContentSizeFitter" => Component::SizeFitter {
                fit: [i32_of(&f, "m_HorizontalFit")?, i32_of(&f, "m_VerticalFit")?],
            },
            "AspectRatioFitter" => Component::AspectFitter {
                mode: i32_of(&f, "m_AspectMode")?,
                ratio: f32_of(&f, "m_AspectRatio")?,
            },
            "uGUI_CircularBar" => Component::CircularBar {
                size: f32_of(&f, "_size")?,
            },
            "ScrollRect" => {
                // Visibility 2 (AutoHideAndExpandViewport) resizes the
                // viewport: not ported.
                for v in [
                    "m_HorizontalScrollbarVisibility",
                    "m_VerticalScrollbarVisibility",
                ] {
                    if i32_of(&f, v).unwrap_or(0) == 2 {
                        notes.push(format!("{path}: ScrollRect {v} 2 is not ported"));
                    }
                }
                Component::ScrollRect
            }
            other => {
                let [element, controller, group, ignorer] = UNPORTED
                    .iter()
                    .find(|(n, _)| *n == other)
                    .map_or([false; 4], |(_, k)| *k);
                Component::Unported {
                    name: other.to_string(),
                    element,
                    controller,
                    group,
                    ignorer,
                }
            }
        };
        Ok(Some(Comp { kind, enabled }))
    }

    /// The layout tree of `tree`: every node's RectTransform as stored
    /// and its layout components.
    pub fn ui_layout(&self, layouts: &Layouts, tree: &UiTree) -> Result<UiLayout> {
        let mut nodes: Vec<Node> = Vec::with_capacity(tree.nodes.len());
        let mut notes = Vec::new();
        for n in &tree.nodes {
            let mut rect = RectTransform::default();
            let mut components = Vec::new();
            let mut found_rect = false;
            for c in &n.components {
                if c.class_id == RECT_TRANSFORM {
                    let (_, d) = c.object.data()?;
                    let r = sn_unity::RectTransform::parse(d, c.object.file.file().big_endian)
                        .map_err(|e| format!("{}: RectTransform: {e}", n.path))?;
                    rect = RectTransform {
                        anchor_min: r.anchor_min,
                        anchor_max: r.anchor_max,
                        anchored_position: r.anchored_position,
                        size_delta: r.size_delta,
                        pivot: r.pivot,
                        scale: r.scale,
                    };
                    found_rect = true;
                } else if c.class_id == MONO_BEHAVIOUR {
                    if let Some(k) = self.layout_component(layouts, c, &mut notes, &n.path)? {
                        components.push(k);
                    }
                }
            }
            if !found_rect {
                notes.push(format!("{}: no RectTransform", n.path));
            }
            nodes.push(Node {
                name: n.name.clone(),
                parent: n.parent,
                children: Vec::new(),
                active_self: n.active_self,
                rect,
                components,
            });
        }
        for i in 0..nodes.len() {
            if let Some(p) = nodes[i].parent {
                if let Some(parent) = nodes.get_mut(p) {
                    parent.children.push(i);
                }
            }
        }
        Ok(UiLayout {
            tree: Tree::new(nodes),
            notes,
        })
    }
}
