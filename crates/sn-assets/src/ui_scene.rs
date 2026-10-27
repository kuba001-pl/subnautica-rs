//! The live UI (M9u6a; `docs/formats/ui.md` § The live UI): a UI tree read
//! once from the install ([`UiScene`]) and drawn again for each
//! [`UiState`], the changes the game's scripts make to it through Unity's
//! API. The screens' rules (M9u6b on) write a state each frame; the draw
//! list is rebuilt from it without reading the install again.
//!
//! Each change is applied where uGUI applies it:
//! - `GameObject.SetActive`: the node's `activeSelf` (layout and drawing);
//! - `CanvasGroup.alpha`: the group's alpha;
//! - `CanvasRenderer.SetAlpha`: the graphic's own alpha, multiplied into
//!   the inherited one (`sn_ui::draw`);
//! - `Graphic.enabled`: the graphic is drawn or not, and is a layout
//!   element or not;
//! - `Graphic.color`: an image's colour; a text's `m_fontColor`, as
//!   `Color32` (`(byte)Mathf.Round(Mathf.Clamp01(c) × 255)`, rounding half
//!   to even);
//! - `Image.sprite`: the mesh's sprite and texture and the layout
//!   element's sprite size; `Image.fillAmount` (clamped to 0…1 as its
//!   setter does);
//! - TMP's `text`: the string, as set from code (`TextString`: escapes
//!   are not parsed);
//! - `RectTransform.anchoredPosition`, `localPosition.z`, `localRotation`,
//!   `localScale`.
//!
//! Materials, fonts' materials and sprites are read the first time a draw
//! needs them and kept, so a draw reads nothing else from the install.

use std::collections::HashMap;
use std::path::PathBuf;

use sn_ui::draw::{DrawInput, Graphic, GroupInfo, draw_list, scale_y_in_root_canvas};
use sn_ui::layout::{Component, RectTransform, SpriteSize, Tree, layout_with};
use sn_ui::material::{UiMaterial, fallback_material, tmp_padding};
use sn_ui::mesh::{SpriteInfo, graphic_quad, image_mesh, raw_image_mesh};
use sn_ui::scaler::{Placement, apply_to_tree};
use sn_ui::state::{MaterialValues, SpriteId, UiState, color32};
use sn_ui::text::{
    FontPad, FontSet, MeshOptions, Pads, Source, TextParams, Texts, layout_text, layout_text_with,
    text_meshes_with,
};

use crate::ui_draw::{MaterialTable, font_texture_key, texture_key};
use crate::{
    Assets, FontAssets, GlyphAtlases, Layouts, ObjectRef, Result, UiDraw, UiDrawParts, UiGraphic,
    UiGraphicKind, UiStartState, UiText, UiTree,
};

type Key = (PathBuf, String, i64);

impl From<&UiStartState> for UiState {
    fn from(s: &UiStartState) -> UiState {
        s.state.clone()
    }
}

/// A sprite as the mesh and the layout take it.
#[derive(Clone)]
struct LoadedSprite {
    info: SpriteInfo,
    texture: Option<ObjectRef>,
}

impl LoadedSprite {
    fn size(&self) -> SpriteSize {
        SpriteSize {
            size: self.info.size,
            border: self.info.border,
            pixels_per_unit: self.info.pixels_per_unit,
        }
    }
}

/// A node made by code (M9u7c1): `new GameObject(name)` with the
/// `RectTransform` a script leaves it with, and the graphic it adds
/// (`enabled` or not). A parent comes before its children.
#[derive(Clone)]
pub struct CodeNode {
    pub name: String,
    /// Within the same list.
    pub parent: Option<usize>,
    pub rect: RectTransform,
    pub active_self: bool,
    pub layer: u32,
    pub graphic: Option<(UiGraphicKind, bool)>,
}

/// A UI tree read once, to draw for any state.
pub struct UiScene {
    pub tree: UiTree,
    /// The layout tree as stored (before placement and layout).
    layout: Tree,
    parts: UiDrawParts,
    graphics: Vec<UiGraphic>,
    texts: Vec<UiText>,
    cache: Cache,
    /// The sprites given ids ([`UiScene::sprite_id`]).
    sprite_refs: HashMap<SpriteId, ObjectRef>,
}

/// A state's tree laid out, with what the draw needs of it.
struct Laid {
    tree: Tree,
    groups: HashMap<usize, GroupInfo>,
    sprites: HashMap<usize, Option<LoadedSprite>>,
    params: Vec<TextParams>,
}

/// What a draw reads from the install, kept after the first time.
#[derive(Default)]
struct Cache {
    /// Materials by object.
    materials: HashMap<Key, UiMaterial>,
    /// Each font asset's own material.
    font_materials: HashMap<usize, Option<UiMaterial>>,
    sprites: HashMap<Key, LoadedSprite>,
    /// Sprites as `uGUI_Icon` draws them (M9u7c1).
    icon_sprites: HashMap<Key, (sn_ui::icon::IconSprite, Option<ObjectRef>)>,
    /// Materials made for texts: a text's after `LoadFontAsset` and
    /// `GetPaddingForMaterial`, and fallback ones.
    derived: Vec<Derived>,
    /// A text's material by (shared material, font, extra padding);
    /// `None`: the font asset has no material.
    text_materials: HashMap<(Option<Key>, usize, bool), Option<usize>>,
    /// A fallback material by (text material, fallback font); `None`: the
    /// fallback font asset has no material.
    fallbacks: HashMap<(usize, usize), Option<usize>>,
    /// A sprite's material by (text material, sprite asset), M9u6d1;
    /// `None`: the sprite asset has no material.
    sprite_materials: HashMap<(usize, usize), Option<usize>>,
}

/// A material made for a text, with its padding.
struct Derived {
    material: UiMaterial,
    pad: FontPad,
}

fn pad_of(m: &UiMaterial, padding: f32) -> FontPad {
    FontPad {
        padding,
        gradient_scale: m.has("_GradientScale").then(|| m.float("_GradientScale")),
        scale_ratio_a: m.has("_ScaleRatioA").then(|| m.float("_ScaleRatioA")),
    }
}

impl Assets<'_> {
    /// Reads `tree`'s layout, draw parts, graphics and texts once
    /// (`sorting_layers`: the TagManager's list).
    pub fn ui_scene(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        fonts: &FontAssets,
        sorting_layers: &[(String, u32)],
    ) -> Result<UiScene> {
        Ok(UiScene {
            parts: self.ui_draw_parts(layouts, tree, sorting_layers)?,
            layout: self.ui_layout(layouts, tree)?.tree,
            graphics: self.ui_graphics(layouts, tree)?,
            texts: self.ui_texts(layouts, tree, fonts)?,
            tree: tree.clone(),
            cache: Cache::default(),
            sprite_refs: HashMap::new(),
        })
    }
}

impl Cache {
    fn material(
        &mut self,
        assets: &Assets,
        o: &ObjectRef,
        fonts: &FontAssets,
    ) -> Result<UiMaterial> {
        let key = o.key();
        if let Some(m) = self.materials.get(&key) {
            return Ok(m.clone());
        }
        let m = assets.ui_material(o, fonts)?;
        self.materials.insert(key, m.clone());
        Ok(m)
    }

    fn font_material(
        &mut self,
        assets: &Assets,
        fonts: &FontAssets,
        font: usize,
    ) -> Result<Option<UiMaterial>> {
        if let Some(m) = self.font_materials.get(&font) {
            return Ok(m.clone());
        }
        let m = assets.font_material(fonts, font)?;
        self.font_materials.insert(font, m.clone());
        Ok(m)
    }

    fn sprite(&mut self, assets: &Assets, s: &ObjectRef) -> Result<LoadedSprite> {
        let key = s.key();
        if let Some(l) = self.sprites.get(&key) {
            return Ok(l.clone());
        }
        let (info, texture) = assets.sprite_info(s)?;
        let l = LoadedSprite { info, texture };
        self.sprites.insert(key, l.clone());
        Ok(l)
    }

    fn derive(&mut self, mut material: UiMaterial, extra_padding: bool) -> usize {
        let padding = tmp_padding(&mut material, extra_padding);
        let pad = pad_of(&material, padding);
        self.derived.push(Derived { material, pad });
        self.derived.len() - 1
    }

    /// A text's material (`LoadFontAsset`): its shared material if that
    /// samples the font's atlas, else the font asset's; its ratios and
    /// padding set.
    fn text_material(
        &mut self,
        assets: &Assets,
        fonts: &FontAssets,
        shared: Option<&ObjectRef>,
        font: usize,
        extra_padding: bool,
    ) -> Result<Option<usize>> {
        let key = (shared.map(ObjectRef::key), font, extra_padding);
        if let Some(id) = self.text_materials.get(&key) {
            return Ok(*id);
        }
        let own = self.font_material(assets, fonts, font)?;
        let shared = match shared {
            Some(o) => Some(self.material(assets, o, fonts)?),
            None => None,
        };
        let atlas_key = font_texture_key(font);
        let main = match shared {
            Some(m) if m.textures.get("_MainTex") == Some(&atlas_key) => Some(m),
            _ => own,
        };
        let id = main.map(|m| self.derive(m, extra_padding));
        self.text_materials.insert(key, id);
        Ok(id)
    }

    /// `GetFallbackMaterial` of text material `main` for font `target`.
    fn fallback(
        &mut self,
        assets: &Assets,
        fonts: &FontAssets,
        main: usize,
        target: usize,
        extra_padding: bool,
    ) -> Result<Option<usize>> {
        if let Some(id) = self.fallbacks.get(&(main, target)) {
            return Ok(*id);
        }
        let id = match self.font_material(assets, fonts, target)? {
            Some(t) => {
                let m = &self.derived[main].material;
                let mut fallback = fallback_material(m, &t);
                // The game's copy keeps the text material's name; ours names
                // both for the logs.
                fallback.name = format!("{} (fallback {})", m.name, t.name);
                Some(self.derive(fallback, extra_padding))
            }
            None => None,
        };
        self.fallbacks.insert((main, target), id);
        Ok(id)
    }
}

impl Cache {
    /// The material a sprite of sprite asset `asset` is drawn with in a
    /// text whose material is `main` (M9u6d1): the game's
    /// `FindMatchingMaterial` over the asset's `m_materials` by name, else
    /// its `material`; its ratios and padding set as a sub-mesh's.
    fn sprite_material(
        &mut self,
        assets: &Assets,
        fonts: &FontAssets,
        main: usize,
        asset: usize,
        extra_padding: bool,
    ) -> Result<Option<usize>> {
        if let Some(id) = self.sprite_materials.get(&(main, asset)) {
            return Ok(*id);
        }
        let s = fonts
            .sprite_assets
            .get(asset)
            .ok_or_else(|| format!("sprite asset {asset} not loaded"))?;
        let mut names = Vec::new();
        for m in &s.materials {
            names.push(self.material(assets, m, fonts)?.name);
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let chosen = match sn_ui::text::sprite::find_matching_material(
            &self.derived[main].material.name,
            &refs,
        ) {
            Some(i) => s.materials.get(i),
            None => s.material.as_ref(),
        };
        let id = match chosen {
            Some(o) => {
                let m = self.material(assets, o, fonts)?;
                Some(self.derive(m, extra_padding))
            }
            None => None,
        };
        self.sprite_materials.insert((main, asset), id);
        Ok(id)
    }
}

impl UiScene {
    /// A template made by code (M9u7c1), to [`UiScene::graft`]: `nodes`
    /// with no stored object, no canvas, group or mask, each graphic
    /// maskable (`MaskableGraphic`'s default) and not culled when
    /// transparent (a new `CanvasRenderer`'s). Node 0 is the root.
    pub fn from_code(key: &str, nodes: Vec<CodeNode>) -> Result<UiScene> {
        let bad = nodes.iter().enumerate().any(|(i, n)| match n.parent {
            None => i != 0,
            Some(p) => p >= i,
        });
        if nodes.is_empty() || bad {
            return Err(format!(
                "{key}: nodes made by code need one root first and parents before children"
            ));
        }
        let mut tree = UiTree {
            key: key.into(),
            nodes: Vec::new(),
        };
        let mut layout = Vec::new();
        let mut parts = UiDrawParts::default();
        let mut graphics = Vec::new();
        for (i, n) in nodes.iter().enumerate() {
            let (path, active) = match n.parent {
                Some(p) => (
                    format!("{}/{}", tree.nodes[p].path, n.name),
                    tree.nodes[p].active && n.active_self,
                ),
                None => (n.name.clone(), n.active_self),
            };
            tree.nodes.push(crate::UiNode {
                name: n.name.clone(),
                parent: n.parent,
                path,
                active_self: n.active_self,
                active,
                layer: n.layer,
                components: Vec::new(),
                game_object: None,
            });
            layout.push(sn_ui::layout::Node {
                name: n.name.clone(),
                parent: n.parent,
                children: (i + 1..nodes.len())
                    .filter(|&c| nodes[c].parent == Some(i))
                    .collect(),
                active_self: n.active_self,
                rect: n.rect,
                components: Vec::new(),
            });
            if let Some((kind, enabled)) = &n.graphic {
                parts.maskable.insert(i, true);
                parts.cull_transparent.insert(i, false);
                graphics.push(UiGraphic {
                    node: i,
                    enabled: *enabled,
                    kind: kind.clone(),
                    material: None,
                });
            }
        }
        Ok(UiScene {
            tree,
            layout: Tree::new(layout),
            parts,
            graphics,
            texts: Vec::new(),
            cache: Cache::default(),
            sprite_refs: HashMap::new(),
        })
    }

    /// The id a state uses for sprite `o` (`Image.sprite = o`).
    pub fn sprite_id(&mut self, o: &ObjectRef) -> SpriteId {
        let id = SpriteId(texture_key(o));
        self.sprite_refs.insert(id, o.clone());
        id
    }

    /// `Object.Instantiate(template)` then `SetParent(parent, false)`
    /// (M9u7a): a copy of `template` (a scene read from another root, e.g.
    /// `uGUI.prefab`'s `MessageInstance`) grafted under `parent` as its last
    /// child, its stored values kept and its root named `<name>(Clone)`, as
    /// Unity names it. Its nodes are appended, so no index of this scene
    /// moves; the copy's node `i` is `root + i`. Returns `root`.
    pub fn graft(&mut self, template: &UiScene, parent: usize) -> Result<usize> {
        let (Some(p), true) = (
            self.tree.nodes.get(parent).cloned(),
            template.tree.nodes.len() == template.layout.nodes.len(),
        ) else {
            return Err(format!(
                "graft {}: parent {parent} not a node, or its tree and layout differ",
                template.tree.key
            ));
        };
        let off = self
            .layout
            .graft(&template.layout, parent)
            .ok_or_else(|| format!("graft {}: not one tree", template.tree.key))?;
        let root_name = format!("{}(Clone)", template.tree.nodes[0].name);
        for (i, n) in template.tree.nodes.iter().enumerate() {
            let mut n = n.clone();
            let parent_active = match n.parent {
                Some(q) => self.tree.nodes[q + off].active,
                None => p.active,
            };
            n.parent = Some(n.parent.map_or(parent, |q| q + off));
            n.path = match i {
                0 => format!("{}/{root_name}", p.path),
                _ => format!(
                    "{}/{root_name}{}",
                    p.path,
                    n.path
                        .strip_prefix(&template.tree.nodes[0].path)
                        .unwrap_or(&n.path)
                ),
            };
            if i == 0 {
                n.name = root_name.clone();
            }
            n.active = parent_active && n.active_self;
            self.tree.nodes.push(n);
        }
        self.layout.nodes[off].name = root_name;
        let parts = &template.parts;
        fn shift<V: Copy>(m: &HashMap<usize, V>, off: usize) -> Vec<(usize, V)> {
            m.iter().map(|(k, v)| (k + off, *v)).collect()
        }
        self.parts.canvases.extend(shift(&parts.canvases, off));
        self.parts.groups.extend(shift(&parts.groups, off));
        self.parts.masks.extend(shift(&parts.masks, off));
        self.parts.maskable.extend(shift(&parts.maskable, off));
        self.parts
            .cull_transparent
            .extend(shift(&parts.cull_transparent, off));
        for l in &parts.unknown_layers {
            if !self.parts.unknown_layers.contains(l) {
                self.parts.unknown_layers.push(*l);
            }
        }
        for g in &template.graphics {
            let mut g = g.clone();
            g.node += off;
            if let UiGraphicKind::Compass { ring, .. } = &mut g.kind {
                for l in &mut ring.labels {
                    l.node += off;
                }
            }
            self.graphics.push(g);
        }
        for t in &template.texts {
            let mut t = t.clone();
            t.node += off;
            self.texts.push(t);
        }
        self.sprite_refs
            .extend(template.sprite_refs.iter().map(|(k, v)| (*k, v.clone())));
        Ok(off)
    }

    /// `Transform.SetAsLastSibling` (M9u7a): `node` drawn last among its
    /// siblings from now on. False for the root or an unknown node.
    pub fn set_as_last_sibling(&mut self, node: usize) -> bool {
        // `UiTree` keeps nodes in their stored order; only the layout tree's
        // children (which the draw order follows) change.
        self.layout.set_as_last_sibling(node)
    }

    /// The number of nodes (grafted ones included).
    pub fn node_count(&self) -> usize {
        self.layout.nodes.len()
    }

    /// A node's `RectTransform` as stored.
    pub fn stored_rect(&self, node: usize) -> Option<&RectTransform> {
        self.layout.nodes.get(node).map(|n| &n.rect)
    }

    /// A canvas's sorting layer (its index in the TagManager's list) and
    /// order; `None` if the node has no canvas.
    pub fn canvas_sorting(&self, node: usize) -> Option<(i32, i32)> {
        self.parts
            .canvases
            .get(&node)
            .map(|c| (c.sorting_layer, c.sorting_order))
    }

    /// `Canvas.sortingLayerID` and `sortingOrder` set by a script
    /// (M9u8b3b3: `uGUI_Tooltip.OnUpdate`); false if the node has no
    /// canvas.
    pub fn set_canvas_sorting(&mut self, node: usize, layer: i32, order: i32) -> bool {
        match self.parts.canvases.get_mut(&node) {
            Some(c) => {
                c.sorting_layer = layer;
                c.sorting_order = order;
                true
            }
            None => false,
        }
    }

    /// The nested CanvasGroup alpha at a graphic, with this frame's state.
    pub fn inherited_alpha(&self, node: usize, state: &UiState) -> f32 {
        let mut alpha = 1.0;
        let mut at = Some(node);
        while let Some(i) = at {
            let Some(n) = self.layout.nodes.get(i) else {
                break;
            };
            if let Some(g) = self.parts.groups.get(&i).filter(|g| g.enabled) {
                alpha *= state.group_alpha.get(&i).copied().unwrap_or(g.alpha);
                if g.ignore_parent_groups {
                    break;
                }
            }
            at = n.parent;
        }
        alpha
    }

    /// A text node's parameters as stored (M9u8b3b); `None` if the node
    /// has no text.
    pub fn text_params(&self, node: usize) -> Option<&TextParams> {
        self.texts
            .iter()
            .find(|t| t.node == node)
            .map(|t| &t.params)
    }

    /// Sorting layer ids not in the TagManager.
    pub fn unknown_layers(&self) -> &[i32] {
        &self.parts.unknown_layers
    }

    /// The tree after `state`, the canvases' placements and the layout
    /// system, as a draw lays it out (M9u7b: what a script reads of
    /// `RectTransform.rect`).
    pub fn layout_for(
        &mut self,
        assets: &Assets,
        set: &FontSet,
        placements: &HashMap<usize, Placement>,
        state: &UiState,
    ) -> Result<Tree> {
        Ok(self.laid_out(assets, set, placements, state)?.tree)
    }

    /// A text node's `preferredWidth` and `preferredHeight` with `text`
    /// (M9u7b), in a rect `rect_width` wide; `None` if the node has no
    /// text.
    pub fn preferred_size(
        &self,
        set: &FontSet,
        node: usize,
        text: &str,
        rect_width: f32,
    ) -> Option<[f32; 2]> {
        let t = self.texts.iter().find(|t| t.node == node)?;
        let mut p = t.params.clone();
        p.text = text.to_string();
        p.source = Source::Code;
        let margin_width = rect_width - p.margin[0] - p.margin[2];
        Some(sn_ui::text::preferred_values(set, &p, margin_width))
    }

    fn laid_out(
        &mut self,
        assets: &Assets,
        set: &FontSet,
        placements: &HashMap<usize, Placement>,
        state: &UiState,
    ) -> Result<Laid> {
        let mut tree = self.layout.clone();
        let mut groups = self.parts.groups.clone();
        let mut sprites: HashMap<usize, Option<LoadedSprite>> = HashMap::new();
        for (node, s) in &state.sprite {
            let path = &self.tree.nodes[*node].path;
            let loaded = match s {
                Some(id) => {
                    let o = self
                        .sprite_refs
                        .get(id)
                        .ok_or_else(|| format!("{path}: sprite {id:?} not given out"))?;
                    Some(
                        self.cache
                            .sprite(assets, o)
                            .map_err(|e| format!("{path}: sprite: {e}"))?,
                    )
                }
                None => None,
            };
            sprites.insert(*node, loaded);
        }
        let sizes = sprites
            .iter()
            .map(|(n, s)| (*n, s.as_ref().map(LoadedSprite::size)))
            .collect();
        apply_to_layout(&mut tree, &mut groups, state, &sizes);
        // Parents first, so a nested canvas sees its parent's new size.
        let mut placed: Vec<(&usize, &Placement)> = placements.iter().collect();
        placed.sort_by_key(|(n, _)| **n);
        for (node, p) in placed {
            apply_to_tree(&mut tree, *node, p);
        }
        // The texts' parameters with the scripts' strings and colours.
        let params: Vec<TextParams> = self
            .texts
            .iter()
            .map(|t| {
                let mut p = t.params.clone();
                apply_to_text(&mut p, t.node, state);
                p
            })
            .collect();
        let sizer = Texts {
            fonts: set,
            params: self
                .texts
                .iter()
                .zip(&params)
                .map(|(t, p)| (t.node, p.clone()))
                .collect(),
        };
        layout_with(&mut tree, &sizer);
        Ok(Laid {
            tree,
            groups,
            sprites,
            params,
        })
    }

    /// The draw list for `state`, with canvases placed as `placements`
    /// say (by node, from `sn_ui::scaler::place`; empty: as stored).
    /// Glyphs the texts need go into `atlases`.
    pub fn draw(
        &mut self,
        assets: &Assets,
        fonts: &FontAssets,
        set: &FontSet,
        atlases: &mut GlyphAtlases,
        placements: &HashMap<usize, Placement>,
        state: &UiState,
    ) -> Result<UiDraw> {
        let Laid {
            tree,
            groups,
            sprites,
            params,
        } = self.laid_out(assets, set, placements, state)?;
        let mut materials = MaterialTable::new();
        let mut graphics = Vec::new();
        let mut textures = HashMap::new();
        let finite = |r: sn_ui::layout::Rect| r.width.is_finite() && r.height.is_finite();
        let maskable = |n: usize| self.parts.maskable.get(&n).copied().unwrap_or(true);
        let cull = |n: usize| {
            self.parts
                .cull_transparent
                .get(&n)
                .copied()
                .unwrap_or(false)
        };
        let alpha = |n: usize| state.renderer_alpha.get(&n).copied().unwrap_or(1.0);
        let mut drawn = Vec::new();
        for g in &self.graphics {
            if !state.enabled.get(&g.node).copied().unwrap_or(g.enabled) {
                continue;
            }
            let rect = tree.rect(g.node);
            if !finite(rect) {
                continue;
            }
            let mut kind = g.kind.clone();
            if !apply_to_graphic(&mut kind, g.node, state, &sprites) {
                continue;
            }
            drawn.push((g.node, kind, g.material.clone(), rect));
        }
        for (node, kind, material, rect) in drawn {
            let objects = match &kind {
                UiGraphicKind::Image { texture, .. }
                | UiGraphicKind::RawImage { texture, .. }
                | UiGraphicKind::Compass { texture, .. } => vec![texture.clone()],
                UiGraphicKind::CircularBar {
                    texture, overlay, ..
                } => vec![texture.clone(), overlay.clone()],
                _ => Vec::new(),
            };
            if let UiGraphicKind::Icon {
                sprite,
                color,
                material,
            } = &kind
            {
                // `uGUI_Icon`: its mesh, `mainTexture` (the sprite's page,
                // or white) and its own material instance.
                let loaded = match sprite {
                    Some(id) => {
                        let path = &self.tree.nodes[node].path;
                        let o = self
                            .sprite_refs
                            .get(id)
                            .ok_or_else(|| format!("{path}: sprite {id:?} not given out"))?;
                        let key = o.key();
                        if !self.cache.icon_sprites.contains_key(&key) {
                            let l = assets
                                .icon_sprite(o)
                                .map_err(|e| format!("{path}: sprite: {e}"))?;
                            self.cache.icon_sprites.insert(key.clone(), l);
                        }
                        self.cache.icon_sprites.get(&key)
                    }
                    None => None,
                };
                let texture = match loaded.and_then(|(_, t)| t.as_ref()) {
                    Some(t) => {
                        textures.insert(texture_key(t), t.clone());
                        texture_key(t)
                    }
                    None => 0,
                };
                let mut m = material.clone();
                m.textures.insert("_MainTex".into(), texture);
                if let Some(v) = state.material.get(&node) {
                    apply_material_values(&mut m, v);
                }
                graphics.push(Graphic {
                    node,
                    mesh: sn_ui::icon::icon_mesh(rect, loaded.map(|(s, _)| s), *color),
                    texture,
                    material: materials.intern(m),
                    maskable: maskable(node),
                    cull_transparent: cull(node),
                    alpha: alpha(node),
                    kind: "uGUI_Icon".into(),
                });
                continue;
            }
            for o in objects.into_iter().flatten() {
                textures.insert(texture_key(&o), o);
            }
            let (mesh, texture, kind) = match &kind {
                UiGraphicKind::Image { params, texture } => {
                    let (kind, m) = image_mesh(rect, tree.nodes[node].rect.pivot, params, 100.0);
                    (
                        m,
                        texture.as_ref().map_or(0, texture_key),
                        format!("Image {kind:?}"),
                    )
                }
                UiGraphicKind::RawImage {
                    color,
                    uv_rect,
                    texture,
                } => (
                    raw_image_mesh(rect, *color, *uv_rect, true),
                    texture.as_ref().map_or(0, texture_key),
                    "RawImage".to_string(),
                ),
                // Its own mesh (`OnPopulateMesh`), drawn while it is
                // enabled (`SetVisible(false)` empties it; the rules
                // disable it then).
                UiGraphicKind::Compass { ring, texture } => (
                    ring.mesh(),
                    texture.as_ref().map_or(0, texture_key),
                    "uGUI_Compass".to_string(),
                ),
                UiGraphicKind::CircularBar {
                    bar,
                    texture,
                    overlay,
                } => {
                    // Its own material instance, as its setters write it;
                    // `Graphic`'s quad.
                    let main = texture.as_ref().map_or(0, texture_key);
                    let mut m = bar.material(
                        sn_ui::bar::START_VALUE,
                        main,
                        overlay.as_ref().map_or(0, texture_key),
                        tree.reference_pixels_per_unit,
                    );
                    if let Some(v) = state.material.get(&node) {
                        apply_material_values(&mut m, v);
                    }
                    graphics.push(Graphic {
                        node,
                        mesh: graphic_quad(rect, bar.color),
                        texture: main,
                        material: materials.intern(m),
                        maskable: maskable(node),
                        cull_transparent: cull(node),
                        alpha: alpha(node),
                        kind: "uGUI_CircularBar".into(),
                    });
                    continue;
                }
                _ => continue,
            };
            let material = match &material {
                Some(o) => match materials.by_key.get(&o.key()) {
                    Some(&i) => i,
                    None => {
                        let m = self
                            .cache
                            .material(assets, o, fonts)
                            .map_err(|e| format!("{}: {e}", self.tree.nodes[node].path))?;
                        materials.list.push(m);
                        let i = materials.list.len() - 1;
                        materials.by_key.insert(o.key(), i);
                        i
                    }
                },
                None => 0,
            };
            // A script's values in the graphic's own material instance.
            let material = match state.material.get(&node) {
                Some(v) => {
                    let mut m = materials.list[material].clone();
                    apply_material_values(&mut m, v);
                    materials.intern(m)
                }
                None => material,
            };
            graphics.push(Graphic {
                node,
                mesh,
                texture,
                material,
                maskable: maskable(node),
                cull_transparent: cull(node),
                alpha: alpha(node),
                kind,
            });
        }
        // Each text material once in this draw's table (equal ones share an
        // index, as `MaterialTable::intern` gives).
        let mut ids: HashMap<usize, usize> = HashMap::new();
        let mut interned = |table: &mut MaterialTable, cache: &Cache, id: usize| -> usize {
            *ids.entry(id)
                .or_insert_with(|| table.intern(cache.derived[id].material.clone()))
        };
        let mut unported = Vec::new();
        let mut text_count = 0;
        let mut sub_meshes = 0;
        for (t, p) in self.texts.iter().zip(&params) {
            let node = t.node;
            if !state.enabled.get(&node).copied().unwrap_or(t.enabled) {
                continue;
            }
            let rect = tree.rect(node);
            if !finite(rect) {
                continue;
            }
            let path = self.tree.nodes[node].path.clone();
            let font = p.font;
            let (extra_padding, mapping, vertex_gradient) =
                (t.extra_padding, t.mapping, t.vertex_gradient);
            let shared = t.material.clone();
            let Some(main) = self
                .cache
                .text_material(assets, fonts, shared.as_ref(), font, extra_padding)
                .map_err(|e| format!("{path}: {e}"))?
            else {
                unported.push(format!("{path}: font asset without a material"));
                continue;
            };
            let mut pads = Pads {
                default: self.cache.derived[main].pad,
                fonts: HashMap::new(),
            };
            // The fonts the text's characters come from (the layout's choice
            // does not depend on padding).
            let first = layout_text(set, p, rect, 0.0);
            let mut sub_materials: HashMap<usize, usize> = HashMap::new();
            for c in first.characters.iter().filter(|c| c.visible) {
                if c.font == font || sub_materials.contains_key(&c.font) {
                    continue;
                }
                let Some(fallback) =
                    self.cache
                        .fallback(assets, fonts, main, c.font, extra_padding)?
                else {
                    unported.push(format!(
                        "{path}: fallback font {} without a material",
                        c.font
                    ));
                    continue;
                };
                pads.fonts.insert(c.font, self.cache.derived[fallback].pad);
                sub_materials.insert(c.font, interned(&mut materials, &self.cache, fallback));
            }
            let main_index = interned(&mut materials, &self.cache, main);
            let layout = layout_text_with(set, p, rect, &pads);
            let options = MeshOptions {
                horizontal_mapping: mapping[0],
                vertical_mapping: mapping[1],
                sdf_shader: self.cache.derived[main].material.has("_WeightNormal"),
                lossy_scale_y: scale_y_in_root_canvas(&tree, &self.parts.canvases, node),
            };
            let (meshes, notes) = text_meshes_with(&layout, p, &options, &set.sprites, |f, g| {
                atlases.glyph(set, f, g)
            });
            for n in notes.iter().chain(layout.unported.iter()) {
                unported.push(format!("{path}: {n}"));
            }
            if vertex_gradient {
                unported.push(format!("{path}: vertex gradient"));
            }
            text_count += 1;
            for m in meshes {
                if m.font != font || m.sprite.is_some() {
                    sub_meshes += 1;
                }
                if let Some(a) = m.sprite {
                    // A sprite asset's sub-mesh: its sheet, its material.
                    let Some(id) = self
                        .cache
                        .sprite_material(assets, fonts, main, a, extra_padding)
                        .map_err(|e| format!("{path}: {e}"))?
                    else {
                        unported.push(format!("{path}: sprite asset {a} without a material"));
                        continue;
                    };
                    let material = interned(&mut materials, &self.cache, id);
                    let texture = self.cache.derived[id]
                        .material
                        .textures
                        .get("_MainTex")
                        .copied()
                        .unwrap_or(0);
                    match &fonts.sprite_assets[a].sheet {
                        Some(o) if texture_key(o) == texture => {
                            textures.insert(texture, o.clone());
                        }
                        _ => unported.push(format!(
                            "{path}: sprite material's _MainTex is not its sheet"
                        )),
                    }
                    graphics.push(Graphic {
                        node,
                        mesh: m.mesh,
                        texture,
                        material,
                        maskable: maskable(node),
                        cull_transparent: cull(node),
                        alpha: alpha(node),
                        kind: "Text sprite sub-mesh".into(),
                    });
                    continue;
                }
                let material = if m.font == font {
                    main_index
                } else {
                    sub_materials.get(&m.font).copied().unwrap_or(main_index)
                };
                graphics.push(Graphic {
                    node,
                    mesh: m.mesh,
                    texture: font_texture_key(m.font),
                    material,
                    maskable: maskable(node),
                    cull_transparent: cull(node),
                    alpha: alpha(node),
                    kind: if m.font == font {
                        "Text".into()
                    } else {
                        "Text sub-mesh".into()
                    },
                });
            }
        }
        let input = DrawInput {
            canvases: self.parts.canvases.clone(),
            groups,
            masks: self.parts.masks.clone(),
            graphics,
        };
        Ok(UiDraw {
            list: draw_list(&tree, &input),
            materials: materials.list,
            unknown_layers: self.parts.unknown_layers.clone(),
            unported,
            texts: text_count,
            sub_meshes,
            textures,
        })
    }
}

/// The changes the layout and the draw list's tree see: `SetActive`, the
/// transform, `CanvasGroup.alpha`, and a graphic's `enabled` and sprite
/// size as a layout element (`sprite_sizes`: the new sprites' sizes by
/// node).
fn apply_to_layout(
    tree: &mut Tree,
    groups: &mut HashMap<usize, GroupInfo>,
    state: &UiState,
    sprite_sizes: &HashMap<usize, Option<SpriteSize>>,
) {
    for (node, alpha) in &state.group_alpha {
        if let Some(g) = groups.get_mut(node) {
            g.alpha = *alpha;
        }
    }
    for (node, active) in &state.active {
        if let Some(n) = tree.nodes.get_mut(*node) {
            n.active_self = *active;
        }
    }
    for (node, t) in &state.transform {
        let Some(n) = tree.nodes.get_mut(*node) else {
            continue;
        };
        if let Some(p) = t.anchored_position {
            n.rect.anchored_position = p;
        }
        if let Some(z) = t.z {
            n.rect.z = z;
        }
        if let Some(r) = t.rotation {
            n.rect.rotation = r;
        }
        if let Some(s) = t.scale {
            n.rect.scale = s;
        }
        if let Some(s) = t.size_delta {
            n.rect.size_delta = s;
        }
    }
    for (node, n) in tree.nodes.iter_mut().enumerate() {
        let enabled = state.enabled.get(&node);
        let sprite = sprite_sizes.get(&node);
        if enabled.is_none() && sprite.is_none() {
            continue;
        }
        for c in &mut n.components {
            match &mut c.kind {
                Component::Image { sprite: size, .. } => {
                    if let Some(e) = enabled {
                        c.enabled = *e;
                    }
                    if let Some(s) = sprite {
                        *size = *s;
                    }
                }
                Component::Text => {
                    if let Some(e) = enabled {
                        c.enabled = *e;
                    }
                }
                _ => {}
            }
        }
    }
}

/// A text's string and colour as a script set them.
fn apply_to_text(p: &mut TextParams, node: usize, state: &UiState) {
    if let Some(s) = state.text.get(&node) {
        p.text = s.clone();
        p.source = Source::Code;
    }
    if let Some(c) = state.color.get(&node) {
        p.color = color32(*c);
    }
}

/// An image's or raw image's own changes: sprite, colour, fill amount.
/// False for the other graphics (texts are drawn apart; the game's own
/// graphics wait for their steps).
fn apply_to_graphic(
    kind: &mut UiGraphicKind,
    node: usize,
    state: &UiState,
    sprites: &HashMap<usize, Option<LoadedSprite>>,
) -> bool {
    match kind {
        UiGraphicKind::Image { params, texture } => {
            if let Some(s) = sprites.get(&node) {
                params.sprite = s.as_ref().map(|s| s.info);
                *texture = s.as_ref().and_then(|s| s.texture.clone());
            }
            if let Some(c) = state.color.get(&node) {
                params.color = *c;
            }
            if let Some(f) = state.fill_amount.get(&node) {
                // `Image.fillAmount`'s setter clamps.
                params.fill_amount = f.clamp(0.0, 1.0);
            }
            true
        }
        UiGraphicKind::RawImage { color, uv_rect, .. } => {
            if let Some(c) = state.color.get(&node) {
                *color = *c;
            }
            if let Some(r) = state.uv_rect.get(&node) {
                *uv_rect = *r;
            }
            true
        }
        UiGraphicKind::CircularBar { bar, .. } => {
            if let Some(c) = state.color.get(&node) {
                bar.color = *c;
            }
            true
        }
        UiGraphicKind::Compass { ring, .. } => {
            if let Some(c) = state.color.get(&node) {
                ring.color = *c;
            }
            true
        }
        UiGraphicKind::Icon { sprite, color, .. } => {
            if let Some(s) = state.sprite.get(&node) {
                *sprite = *s;
            }
            if let Some(c) = state.color.get(&node) {
                *color = *c;
            }
            true
        }
        _ => false,
    }
}

/// A script's `SetFloat`, `SetVector`/`SetColor` and keyword calls on a
/// material instance.
fn apply_material_values(m: &mut UiMaterial, v: &MaterialValues) {
    for (k, x) in &v.floats {
        m.floats.insert(k.clone(), *x);
    }
    for (k, x) in &v.vectors {
        m.colors.insert(k.clone(), *x);
    }
    for (k, on) in &v.keywords {
        m.keywords.retain(|w| w != k);
        if *on {
            m.keywords.push(k.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sn_ui::layout::{Comp, Node, RectTransform};
    use sn_ui::state::LocalTransform;

    fn node(
        name: &str,
        parent: Option<usize>,
        children: Vec<usize>,
        comp: Option<Component>,
    ) -> Node {
        Node {
            name: name.into(),
            parent,
            children,
            active_self: true,
            rect: RectTransform::default(),
            components: comp
                .into_iter()
                .map(|kind| Comp {
                    kind,
                    enabled: true,
                })
                .collect(),
        }
    }

    fn sample_tree() -> Tree {
        Tree::new(vec![
            node("canvas", None, vec![1, 2], None),
            node(
                "image",
                Some(0),
                vec![],
                Some(Component::Image {
                    sprite: None,
                    kind: 0,
                }),
            ),
            node("text", Some(0), vec![], Some(Component::Text)),
        ])
    }

    fn sprite(size: f32) -> SpriteInfo {
        SpriteInfo::new(
            [size, size],
            [0.0; 4],
            100.0,
            [0.0, 0.0, size, size],
            [0.0, 0.0],
            [256.0, 256.0],
            false,
            false,
        )
    }

    fn image_params() -> sn_ui::mesh::ImageParams {
        sn_ui::mesh::ImageParams {
            sprite: Some(sprite(32.0)),
            kind: 3,
            preserve_aspect: false,
            fill_center: true,
            fill_method: 4,
            fill_amount: 1.0,
            fill_clockwise: true,
            fill_origin: 2,
            use_sprite_mesh: false,
            pixels_per_unit_multiplier: 1.0,
            color: [1.0; 4],
        }
    }

    #[test]
    fn layout_changes_reach_the_tree_and_groups() {
        let mut tree = sample_tree();
        let mut groups: HashMap<usize, GroupInfo> = [(
            0,
            GroupInfo {
                alpha: 1.0,
                ignore_parent_groups: false,
                enabled: true,
            },
        )]
        .into_iter()
        .collect();
        let mut state = UiState::default();
        state.active.insert(2, false);
        state.group_alpha.insert(0, 0.25);
        // A node without a CanvasGroup gets none.
        state.group_alpha.insert(1, 0.5);
        state.enabled.insert(1, false);
        state.enabled.insert(2, false);
        let q = [0.0, 1.0, 0.0, 0.0];
        state.transform.insert(
            1,
            LocalTransform {
                anchored_position: Some([3.0, 4.0]),
                z: Some(-1.0),
                rotation: Some(q),
                scale: Some([2.0, 2.0, 1.0]),
                size_delta: Some([12.0, 34.0]),
            },
        );
        let size = SpriteSize {
            size: [64.0, 32.0],
            border: [0.0; 4],
            pixels_per_unit: 100.0,
        };
        let sizes = [(1, Some(size))].into_iter().collect();
        apply_to_layout(&mut tree, &mut groups, &state, &sizes);
        assert!(!tree.nodes[2].active_self);
        assert!(tree.nodes[1].active_self);
        assert_eq!(groups[&0].alpha, 0.25);
        assert!(!groups.contains_key(&1));
        let r = &tree.nodes[1].rect;
        assert_eq!(r.anchored_position, [3.0, 4.0]);
        assert_eq!(r.z, -1.0);
        assert_eq!(r.rotation, q);
        assert_eq!(r.scale, [2.0, 2.0, 1.0]);
        assert_eq!(r.size_delta, [12.0, 34.0]);
        assert!(!tree.nodes[1].components[0].enabled);
        assert!(!tree.nodes[2].components[0].enabled);
        assert_eq!(
            tree.nodes[1].components[0].kind,
            Component::Image {
                sprite: Some(size),
                kind: 0
            }
        );
        // An empty state changes nothing.
        let before = sample_tree();
        let mut t = sample_tree();
        apply_to_layout(
            &mut t,
            &mut HashMap::new(),
            &UiState::default(),
            &HashMap::new(),
        );
        assert_eq!(t, before);
    }

    #[test]
    fn a_texts_string_is_set_from_code_and_its_colour_as_color32() {
        let mut p = TextParams::new("stored", 0, 36.0);
        p.source = Source::Stored;
        let mut state = UiState::default();
        apply_to_text(&mut p, 7, &state);
        assert_eq!(p.text, "stored");
        assert_eq!(p.source, Source::Stored);
        state.text.insert(7, "42".into());
        state.color.insert(7, [1.0, 0.5, 0.0, 1.0]);
        apply_to_text(&mut p, 7, &state);
        assert_eq!(p.text, "42");
        assert_eq!(p.source, Source::Code);
        assert_eq!(p.color, [255, 128, 0, 255]);
    }

    #[test]
    fn an_images_sprite_colour_and_fill_change() {
        let mut kind = UiGraphicKind::Image {
            params: image_params(),
            texture: None,
        };
        let mut state = UiState::default();
        state.fill_amount.insert(1, 1.5);
        state.color.insert(1, [0.0, 1.0, 0.0, 0.5]);
        let other = LoadedSprite {
            info: sprite(64.0),
            texture: None,
        };
        let sprites = [(1, Some(other))].into_iter().collect();
        assert!(apply_to_graphic(&mut kind, 1, &state, &sprites));
        let UiGraphicKind::Image { params, .. } = &kind else {
            unreachable!()
        };
        assert_eq!(params.fill_amount, 1.0);
        assert_eq!(params.color, [0.0, 1.0, 0.0, 0.5]);
        assert_eq!(params.sprite.map(|s| s.size), Some([64.0, 64.0]));
        state.fill_amount.insert(1, -0.5);
        let none = [(1, None)].into_iter().collect();
        assert!(apply_to_graphic(&mut kind, 1, &state, &none));
        let UiGraphicKind::Image { params, texture } = &kind else {
            unreachable!()
        };
        assert_eq!(params.fill_amount, 0.0);
        assert!(params.sprite.is_none() && texture.is_none());
        // A raw image takes the colour; a text is not drawn here.
        let mut raw = UiGraphicKind::RawImage {
            color: [1.0; 4],
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            texture: None,
        };
        assert!(apply_to_graphic(&mut raw, 1, &state, &HashMap::new()));
        assert!(
            matches!(raw, UiGraphicKind::RawImage { color, .. } if color == [0.0, 1.0, 0.0, 0.5])
        );
        assert!(!apply_to_graphic(
            &mut UiGraphicKind::Text,
            1,
            &state,
            &HashMap::new()
        ));
    }

    #[test]
    fn the_start_state_becomes_a_state() {
        let mut s = UiStartState::default();
        s.state.group_alpha.insert(4, 0.0);
        s.state.active.insert(5, false);
        s.state.enabled.insert(6, false);
        s.state.renderer_alpha.insert(7, 0.0);
        let u = UiState::from(&s);
        assert_eq!(u.group_alpha[&4], 0.0);
        assert!(!u.active[&5]);
        assert!(!u.enabled[&6]);
        assert_eq!(u.renderer_alpha[&7], 0.0);
        assert!(u.text.is_empty());
    }
}
