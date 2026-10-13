//! A UI hierarchy as `sn-ui`'s layout reads it (M9u4a): each node's
//! RectTransform and its layout components, their fields read by the
//! layouts from the game's assemblies (M9u1).

use sn_ui::layout::{Comp, Component, HvGroup, Node, Padding, RectTransform, SpriteSize, Tree};
use sn_unity::{Fields, MonoBehaviourHeader, Sprite, Value};

use crate::{Assets, Layouts, ObjectRef, Result, UiComponent, UiTree};

const RECT_TRANSFORM: i32 = 224;
const TRANSFORM: i32 = 4;
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
    pub(crate) fn script_fields(
        &self,
        layouts: &Layouts,
        c: &UiComponent,
    ) -> Result<(bool, Fields)> {
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
                if c.class_id == TRANSFORM {
                    // A plain Transform: no rect (its children see a parent
                    // size of 0), placed at its own position.
                    let (_, d) = c.object.data()?;
                    let t = sn_unity::TransformNode::parse(d, c.object.file.file().big_endian)
                        .map_err(|e| format!("{}: Transform: {e}", n.path))?;
                    rect = RectTransform {
                        anchor_min: [0.0, 0.0],
                        anchor_max: [0.0, 0.0],
                        anchored_position: [t.position[0], t.position[1]],
                        size_delta: [0.0, 0.0],
                        pivot: [0.0, 0.0],
                        scale: t.scale,
                        rotation: t.rotation,
                        z: t.position[2],
                    };
                    found_rect = true;
                } else if c.class_id == RECT_TRANSFORM {
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
                        rotation: r.rotation,
                        z: r.position[2],
                    };
                    found_rect = true;
                } else if c.class_id == MONO_BEHAVIOUR {
                    if let Some(k) = self.layout_component(layouts, c, &mut notes, &n.path)? {
                        components.push(k);
                    }
                }
            }
            if !found_rect {
                notes.push(format!("{}: no Transform", n.path));
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

/// A graphic of a UI node, as its mesh needs it (M9u4b).
#[derive(Clone)]
pub enum UiGraphicKind {
    Image {
        params: sn_ui::mesh::ImageParams,
        /// The texture its sprite is drawn from (its own or the atlas
        /// page).
        texture: Option<ObjectRef>,
    },
    RawImage {
        color: [f32; 4],
        uv_rect: [f32; 4],
        texture: Option<ObjectRef>,
    },
    /// A TMP text (its mesh is M9u4c's).
    Text,
    /// Another graphic (the game's own `Graphic` subclasses), by script.
    Other(String),
}

#[derive(Clone)]
pub struct UiGraphic {
    pub node: usize,
    pub enabled: bool,
    pub kind: UiGraphicKind,
    /// `m_Material` (none: the canvas's default, `UI/Default`).
    pub material: Option<ObjectRef>,
}

/// The game's own graphics: its classes that extend `Graphic` or
/// `MaskableGraphic` (`sn-inspect code --extends Graphic`), drawn by their
/// own code (M9u6, M9u8b).
const OTHER_GRAPHICS: [&str; 8] = [
    "uGUI_Block",
    "uGUI_CircularBar",
    "uGUI_Compass",
    "uGUI_EquipmentSlot",
    "uGUI_ItemIcon",
    "uGUI_ListItem",
    "uGUI_Logo",
    "uGUI_Icon",
];

fn color_of(f: &Fields, name: &str) -> Result<[f32; 4]> {
    let c = f
        .get(name)
        .and_then(Value::as_struct)
        .ok_or_else(|| format!("no colour {name}"))?;
    Ok([
        f32_of(c, "r")?,
        f32_of(c, "g")?,
        f32_of(c, "b")?,
        f32_of(c, "a")?,
    ])
}

impl Assets<'_> {
    /// A sprite's mesh data and the texture it is drawn from: its own
    /// render data, or (for a sprite packed late, whose own texture is
    /// null) its atlas's render data for it.
    pub fn sprite_info(
        &self,
        sprite: &ObjectRef,
    ) -> Result<(sn_ui::mesh::SpriteInfo, Option<ObjectRef>)> {
        let (info, d) = sprite.data()?;
        if info.class_id != SPRITE {
            return Err(format!("class {} is not a Sprite", info.class_id));
        }
        let s =
            Sprite::parse(d, sprite.file.file().big_endian).map_err(|e| format!("Sprite: {e}"))?;
        let mut rd = s.rd.clone();
        if rd.texture.is_null() && !s.sprite_atlas.is_null() {
            if let Some(a) = self.resolve(&sprite.file, s.sprite_atlas)? {
                let (_, ad) = a.data()?;
                let atlas = sn_unity::SpriteAtlas::parse(ad, a.file.file().big_endian)
                    .map_err(|e| format!("SpriteAtlas: {e}"))?;
                if let Some(packed) = atlas.render_data_of(&s.render_data_key) {
                    rd = packed.clone();
                    let page = self.resolve(&a.file, rd.texture)?;
                    return self.sprite_info_with(&s, &rd, page);
                }
            }
        }
        let page = self.resolve(&sprite.file, rd.texture)?;
        self.sprite_info_with(&s, &rd, page)
    }

    fn sprite_info_with(
        &self,
        s: &Sprite,
        rd: &sn_unity::SpriteRenderData,
        page: Option<ObjectRef>,
    ) -> Result<(sn_ui::mesh::SpriteInfo, Option<ObjectRef>)> {
        let (size, repeat) = match &page {
            Some(p) => {
                let (_, td) = p.data()?;
                let t = sn_unity::Texture2D::parse(td, p.file.file().big_endian)
                    .map_err(|e| format!("Texture2D: {e}"))?;
                ([t.width as f32, t.height as f32], t.wrap[0] == 0)
            }
            None => ([1.0, 1.0], false),
        };
        Ok((
            sn_ui::mesh::SpriteInfo::new(
                [s.rect[2], s.rect[3]],
                s.border,
                s.pixels_to_units,
                rd.texture_rect,
                rd.texture_rect_offset,
                size,
                rd.packed(),
                repeat,
            ),
            page,
        ))
    }

    /// Every graphic of a UI tree, in node order.
    pub fn ui_graphics(&self, layouts: &Layouts, tree: &UiTree) -> Result<Vec<UiGraphic>> {
        let mut out = Vec::new();
        for (node, c) in tree.components() {
            let Some(script) = c.script.as_deref() else {
                continue;
            };
            let known = matches!(script, "Image" | "RawImage" | "TextMeshProUGUI")
                || OTHER_GRAPHICS.contains(&script);
            if !known {
                continue;
            }
            let (enabled, f) = self.script_fields(layouts, c)?;
            let path = &tree.nodes[node].path;
            let kind = match script {
                "Image" => {
                    let (sprite, texture) = match f.get("m_Sprite").and_then(Value::as_pptr) {
                        Some(p) => match self.resolve(&c.object.file, p)? {
                            Some(o) => {
                                let (s, t) =
                                    self.sprite_info(&o).map_err(|e| format!("{path}: {e}"))?;
                                (Some(s), t)
                            }
                            None => (None, None),
                        },
                        None => (None, None),
                    };
                    UiGraphicKind::Image {
                        params: sn_ui::mesh::ImageParams {
                            sprite,
                            kind: i32_of(&f, "m_Type")?,
                            preserve_aspect: bool_of(&f, "m_PreserveAspect")?,
                            fill_center: bool_of(&f, "m_FillCenter")?,
                            fill_method: i32_of(&f, "m_FillMethod")?,
                            fill_amount: f32_of(&f, "m_FillAmount")?,
                            fill_clockwise: bool_of(&f, "m_FillClockwise")?,
                            fill_origin: i32_of(&f, "m_FillOrigin")?,
                            use_sprite_mesh: bool_of(&f, "m_UseSpriteMesh")?,
                            pixels_per_unit_multiplier: f32_of(&f, "m_PixelsPerUnitMultiplier")?,
                            color: color_of(&f, "m_Color")?,
                        },
                        texture,
                    }
                }
                "RawImage" => {
                    let uv = f
                        .get("m_UVRect")
                        .and_then(Value::as_struct)
                        .ok_or("no m_UVRect")?;
                    let texture = match f.get("m_Texture").and_then(Value::as_pptr) {
                        Some(p) => self.resolve(&c.object.file, p)?,
                        None => None,
                    };
                    UiGraphicKind::RawImage {
                        color: color_of(&f, "m_Color")?,
                        uv_rect: [
                            f32_of(uv, "x")?,
                            f32_of(uv, "y")?,
                            f32_of(uv, "width")?,
                            f32_of(uv, "height")?,
                        ],
                        texture,
                    }
                }
                "TextMeshProUGUI" => UiGraphicKind::Text,
                other => UiGraphicKind::Other(other.to_string()),
            };
            let material = match f.get("m_Material").and_then(Value::as_pptr) {
                Some(p) if !p.is_null() => self.resolve(&c.object.file, p)?,
                _ => None,
            };
            out.push(UiGraphic {
                node,
                enabled,
                kind,
                material,
            });
        }
        Ok(out)
    }
}

/// What the draw list reads from a UI tree besides its layout and
/// graphics (M9u4d): canvases, canvas groups, rect masks, and each
/// graphic's `m_Maskable` and `CanvasRenderer.cullTransparentMesh`.
#[derive(Default)]
pub struct UiDrawParts {
    pub canvases: std::collections::HashMap<usize, sn_ui::draw::CanvasInfo>,
    pub groups: std::collections::HashMap<usize, sn_ui::draw::GroupInfo>,
    pub masks: std::collections::HashMap<usize, sn_ui::draw::MaskInfo>,
    /// Graphic nodes and whether they are maskable (one graphic a node).
    pub maskable: std::collections::HashMap<usize, bool>,
    pub cull_transparent: std::collections::HashMap<usize, bool>,
    /// Sorting layer ids not in the TagManager.
    pub unknown_layers: Vec<i32>,
}

const CANVAS: i32 = 223;
const CANVAS_GROUP: i32 = 225;
const CANVAS_RENDERER: i32 = 222;

impl Assets<'_> {
    /// The draw parts of `tree`; `sorting_layers` is the TagManager's list
    /// (name, unique id) in order.
    pub fn ui_draw_parts(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        sorting_layers: &[(String, u32)],
    ) -> Result<UiDrawParts> {
        let mut out = UiDrawParts::default();
        for (node, c) in tree.components() {
            let path = &tree.nodes[node].path;
            match c.class_id {
                CANVAS => {
                    let (_, d) = c.object.data()?;
                    let cv = sn_unity::Canvas::parse(d, c.object.file.file().big_endian)
                        .map_err(|e| format!("{path}: Canvas: {e}"))?;
                    let layer = sorting_layers
                        .iter()
                        .position(|(_, id)| *id as i32 == cv.sorting_layer_id);
                    if layer.is_none() {
                        out.unknown_layers.push(cv.sorting_layer_id);
                    }
                    out.canvases.insert(
                        node,
                        sn_ui::draw::CanvasInfo {
                            enabled: cv.enabled,
                            override_sorting: cv.override_sorting,
                            sorting_layer: layer.map_or(0, |l| l as i32),
                            sorting_order: i32::from(cv.sorting_order),
                        },
                    );
                }
                CANVAS_GROUP => {
                    let (_, d) = c.object.data()?;
                    let g = sn_unity::CanvasGroup::parse(d, c.object.file.file().big_endian)
                        .map_err(|e| format!("{path}: CanvasGroup: {e}"))?;
                    out.groups.insert(
                        node,
                        sn_ui::draw::GroupInfo {
                            alpha: g.alpha,
                            ignore_parent_groups: g.ignore_parent_groups,
                            enabled: g.enabled,
                        },
                    );
                }
                CANVAS_RENDERER => {
                    let (_, d) = c.object.data()?;
                    let r = sn_unity::CanvasRenderer::parse(d, c.object.file.file().big_endian)
                        .map_err(|e| format!("{path}: CanvasRenderer: {e}"))?;
                    out.cull_transparent.insert(node, r.cull_transparent_mesh);
                }
                MONO_BEHAVIOUR => {
                    let Some(script) = c.script.as_deref() else {
                        continue;
                    };
                    if script == "RectMask2D" {
                        let (enabled, f) = self.script_fields(layouts, c)?;
                        let p = f
                            .get("m_Padding")
                            .and_then(Value::as_struct)
                            .ok_or("no m_Padding")?;
                        let s = f
                            .get("m_Softness")
                            .and_then(Value::as_struct)
                            .ok_or("no m_Softness")?;
                        out.masks.insert(
                            node,
                            sn_ui::draw::MaskInfo {
                                enabled,
                                padding: [
                                    f32_of(p, "x")?,
                                    f32_of(p, "y")?,
                                    f32_of(p, "z")?,
                                    f32_of(p, "w")?,
                                ],
                                softness: [i32_of(s, "m_X")?, i32_of(s, "m_Y")?],
                            },
                        );
                    } else if matches!(script, "Image" | "RawImage" | "TextMeshProUGUI") {
                        let (_, f) = self.script_fields(layouts, c)?;
                        out.maskable
                            .insert(node, bool_of(&f, "m_Maskable").unwrap_or(true));
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }
}
