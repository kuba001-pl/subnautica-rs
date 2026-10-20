//! What `uGUI_PDA` reads (M9u8a4a; `docs/formats/ui.md` § The PDA's
//! screen): the screen prefab (`PDA.prefabScreen` in the main scene) and
//! the nodes its states change ([`sn_ui::pda::PdaScreenConfig`]); its
//! `uGUI_Toolbar` (M9u8a4b, [`PdaToolbar`]) and the icons it makes by
//! code ([`toolbar_icon_nodes`]).

use std::collections::{BTreeMap, HashMap};

use sn_ui::icon::NEW_RECT_SIZE;
use sn_ui::inventory_tab::{ContainerNodes, InventoryTabConfig, SlotNodes};
use sn_ui::layout::{RectTransform, V2};
use sn_ui::material::UiMaterial;
use sn_ui::pda::{PdaScreenConfig, Tab, TabNodes, TabShow};
use sn_ui::toolbar::ToolbarConfig;
use sn_unity::{Fields, PPtr, Sprite, Value};

use crate::icons::foreground_material;
use crate::ui_scene::CodeNode;
use crate::{Assets, Layouts, ObjectRef, Result, UiGraphicKind, UiTree};

const SPRITE: i32 = 213;
const TOOLBAR: &str = "uGUI_Toolbar";

/// `uGUI_Toolbar` as stored: its config (the layout fields, 0 here, are
/// set by [`ToolbarConfig`]'s caller from a laid-out tree), its two
/// background sprites with their `rect` sizes, and the notification
/// label's place (logged; no count is drawn yet).
#[derive(Clone, Debug)]
pub struct PdaToolbar {
    pub config: ToolbarConfig,
    pub normal: (ObjectRef, String, V2),
    pub selected: (ObjectRef, String, V2),
    pub notification_anchor: i32,
    pub notification_offset: V2,
}

fn vec2(f: &Fields, name: &str) -> Result<V2> {
    let v = f
        .get(name)
        .and_then(Value::as_struct)
        .ok_or_else(|| format!("{TOOLBAR}.{name}: not read"))?;
    let c = |k: &str| {
        v.get(k)
            .and_then(Value::as_f64)
            .map(|x| x as f32)
            .ok_or_else(|| format!("{TOOLBAR}.{name}.{k}: not read"))
    };
    Ok([c("x")?, c("y")?])
}

const TYPE: &str = "uGUI_PDA";

/// `uGUI_PDA`'s tab fields, in `Initialize`'s dictionary order.
const TAB_FIELDS: [(&str, Tab); 8] = [
    ("tabIntro", Tab::Intro),
    ("tabInventory", Tab::Inventory),
    ("tabJournal", Tab::Journal),
    ("tabPing", Tab::Ping),
    ("tabGallery", Tab::Gallery),
    ("tabLog", Tab::Log),
    ("tabEncyclopedia", Tab::Encyclopedia),
    ("tabTimeCapsule", Tab::TimeCapsule),
];

fn pptr(f: &Fields, name: &str) -> Result<PPtr> {
    f.get(name)
        .and_then(Value::as_pptr)
        .ok_or_else(|| format!("{name}: not read"))
}

/// The node holding the component `p` points at (from `from`'s file).
fn node_of(assets: &Assets, tree: &UiTree, from: &ObjectRef, p: PPtr, what: &str) -> Result<usize> {
    let o = assets
        .resolve(&from.file, p)?
        .ok_or_else(|| format!("{what}: not set"))?;
    tree.nodes
        .iter()
        .position(|n| n.components.iter().any(|c| c.object.key() == o.key()))
        .ok_or_else(|| format!("{what}: not in the tree"))
}

/// `uGUI_Toolbar.CreateIcon`'s icon, as made by code: `new
/// GameObject("ToolbarIcon")` with a `uGUI_ItemIcon` (`Init`'s
/// `SetParams(anchor (0, 0), pivot (0.5, 0.5))` on a new `RectTransform`),
/// and the `Background` and `Foreground` `uGUI_Icon`s `CreateBackground` /
/// `CreateForeground` make (disabled, anchors and pivot at the centre,
/// each with its own copy of `iconMaterial`: `GRAYSCALE` and
/// `NOTIFICATION` off), the background first (`RearrangeElements`).
/// Node 0 is the icon, 1 the background, 2 the foreground.
pub fn toolbar_icon_nodes(layer: u32, material: &UiMaterial) -> Vec<CodeNode> {
    let rect = |anchor: [f32; 2]| RectTransform {
        anchor_min: anchor,
        anchor_max: anchor,
        anchored_position: [0.0, 0.0],
        size_delta: NEW_RECT_SIZE,
        pivot: [0.5, 0.5],
        scale: [1.0, 1.0, 1.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        z: 0.0,
    };
    let icon = |name: &str| CodeNode {
        name: name.into(),
        parent: Some(0),
        rect: rect([0.5, 0.5]),
        active_self: true,
        layer,
        graphic: Some((
            UiGraphicKind::Icon {
                sprite: None,
                color: [1.0; 4],
                material: foreground_material(material),
            },
            false,
        )),
    };
    vec![
        CodeNode {
            name: "ToolbarIcon".into(),
            parent: None,
            rect: rect([0.0, 0.0]),
            active_self: true,
            layer,
            graphic: None,
        },
        icon("Background"),
        icon("Foreground"),
    ]
}

/// `uGUI_InventoryTab`'s language keys (`OnLanguageChanged`, the
/// equipment's label, the default storage label).
pub const INVENTORY_TAB_KEYS: [&str; 3] = ["InventoryLabel", "StorageLabel", "EquipmentLabel"];

/// The node whose GameObject `p` points at (from `from`'s file).
fn game_object_node(
    assets: &Assets,
    tree: &UiTree,
    from: &ObjectRef,
    p: PPtr,
    what: &str,
) -> Result<usize> {
    let o = assets
        .resolve(&from.file, p)?
        .ok_or_else(|| format!("{what}: not set"))?;
    tree.nodes
        .iter()
        .position(|n| n.game_object.as_ref().is_some_and(|g| g.key() == o.key()))
        .ok_or_else(|| format!("{what}: not in the tree"))
}

fn below(tree: &UiTree, mut node: usize, ancestor: usize) -> bool {
    let mut guard = 0;
    while let Some(p) = tree.nodes[node].parent {
        if p == ancestor {
            return true;
        }
        node = p;
        guard += 1;
        if guard > tree.nodes.len() {
            return false;
        }
    }
    false
}

impl Assets<'_> {
    /// `uGUI_InventoryTab` in `tree` (the screen prefab): its containers
    /// (`inventory`, `storage`, `torpedoStorage`, each with its `grid`),
    /// `equipment` and its named `uGUI_EquipmentSlot`s (hierarchy order:
    /// `GetComponentsInChildren`), the two labels; `language` gives the
    /// texts of [`INVENTORY_TAB_KEYS`] and `extra_keys` (the locker's
    /// label). The containers' layout fields are 0 until
    /// [`InventoryTabConfig::lay_out`].
    pub fn inventory_tab_config(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        language: &HashMap<String, String>,
        extra_keys: &[&str],
    ) -> Result<InventoryTabConfig> {
        const TAB: &str = "uGUI_InventoryTab";
        let (_, c) = tree.scripts(TAB).next().ok_or("no uGUI_InventoryTab")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let e = |m: String| format!("{TAB}.{m}");
        let node = |name: &str| {
            node_of(
                self,
                tree,
                &c.object,
                pptr(&f, name).map_err(e)?,
                &e(name.into()),
            )
        };
        let container = |p: PPtr, what: String| -> Result<ContainerNodes> {
            let n = node_of(self, tree, &c.object, p, &what)?;
            let script = self
                .resolve(&c.object.file, p)?
                .ok_or_else(|| format!("{what}: not set"))?;
            let (_, cf) = self.object_fields(layouts, &script)?;
            let grid = match pptr(&cf, "grid") {
                Ok(g) if !g.is_null() => {
                    Some(node_of(self, tree, &script, g, &format!("{what}.grid"))?)
                }
                _ => None,
            };
            Ok(ContainerNodes::new(n, grid))
        };
        let inventory = container(pptr(&f, "inventory").map_err(e)?, e("inventory".into()))?;
        let storage = container(pptr(&f, "storage").map_err(e)?, e("storage".into()))?;
        let mut torpedo = Vec::new();
        for (k, v) in f
            .get("torpedoStorage")
            .and_then(Value::as_array)
            .ok_or_else(|| e("torpedoStorage: not read".into()))?
            .iter()
            .enumerate()
        {
            let p = v
                .as_pptr()
                .ok_or_else(|| e(format!("torpedoStorage[{k}]: not a reference")))?;
            if !p.is_null() {
                torpedo.push(container(p, e(format!("torpedoStorage[{k}]")))?);
            }
        }
        let equipment = node("equipment")?;
        let mut slots = Vec::new();
        for (n, sc) in tree.scripts("uGUI_EquipmentSlot") {
            if !below(tree, n, equipment) {
                continue;
            }
            let (_, sf) = self.script_fields(layouts, sc)?;
            let what = format!("uGUI_EquipmentSlot {}", tree.nodes[n].path);
            let slot = sf
                .get("slot")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{what}.slot: not read"))?;
            if slot.is_empty() {
                continue;
            }
            let background = node_of(
                self,
                tree,
                &sc.object,
                pptr(&sf, "background").map_err(|m| format!("{what}.{m}"))?,
                &format!("{what}.background"),
            )?;
            let hint = match pptr(&sf, "hint") {
                Ok(h) if !h.is_null() => Some(game_object_node(
                    self,
                    tree,
                    &sc.object,
                    h,
                    &format!("{what}.hint"),
                )?),
                _ => None,
            };
            let icon_rect = node_of(
                self,
                tree,
                &sc.object,
                pptr(&sf, "iconRect").map_err(|m| format!("{what}.{m}"))?,
                &format!("{what}.iconRect"),
            )?;
            slots.push(SlotNodes {
                slot: slot.to_string(),
                node: n,
                background,
                hint,
                icon_rect,
                icon_size: [0.0, 0.0],
            });
        }
        let mut texts = BTreeMap::new();
        for k in INVENTORY_TAB_KEYS.iter().chain(extra_keys) {
            if let Some(v) = language.get(*k) {
                texts.insert(k.to_string(), v.clone());
            }
        }
        Ok(InventoryTabConfig {
            inventory,
            storage,
            torpedo,
            equipment,
            slots,
            inventory_label: node("inventoryLabel")?,
            storage_label: node("storageLabel")?,
            language: texts,
        })
    }

    /// `uGUI_PDA.toolbar` in `tree` (the screen prefab): its node,
    /// `selectionMarker`'s (which must be the toolbar's child: `Select`
    /// sets its `localPosition`), the stored sizes and the background
    /// sprites.
    pub fn pda_toolbar(&self, layouts: &Layouts, tree: &UiTree) -> Result<PdaToolbar> {
        let (_, c) = tree.scripts(TOOLBAR).next().ok_or("no uGUI_Toolbar")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let node = tree
            .nodes
            .iter()
            .position(|n| {
                n.components
                    .iter()
                    .any(|k| k.object.key() == c.object.key())
            })
            .ok_or("uGUI_Toolbar: not in the tree")?;
        let e = |m: String| format!("{TOOLBAR}.{m}");
        let marker = match pptr(&f, "selectionMarker").map_err(e)? {
            p if p.is_null() => None,
            p => {
                let m = node_of(self, tree, &c.object, p, &e("selectionMarker".into()))?;
                if tree.nodes[m].parent != Some(node) {
                    return Err(e(format!(
                        "selectionMarker: {} is not the toolbar's child",
                        tree.nodes[m].path
                    )));
                }
                Some(m)
            }
        };
        let sprite = |name: &str| -> Result<(ObjectRef, String, V2)> {
            let o = self
                .resolve(&c.object.file, pptr(&f, name).map_err(e)?)?
                .ok_or_else(|| e(format!("{name}: not set")))?;
            let (info, d) = o.data()?;
            if info.class_id != SPRITE {
                return Err(e(format!("{name}: class {}", info.class_id)));
            }
            let s = Sprite::parse(d, o.file.file().big_endian)
                .map_err(|m| e(format!("{name}: Sprite: {m}")))?;
            Ok((o, s.name, [s.rect[2], s.rect[3]]))
        };
        let f32_of = |name: &str| {
            f.get(name)
                .and_then(Value::as_f64)
                .map(|x| x as f32)
                .ok_or_else(|| e(format!("{name}: not read")))
        };
        Ok(PdaToolbar {
            config: ToolbarConfig {
                node,
                marker,
                background_size: vec2(&f, "backgroundSize")?,
                foreground_size: vec2(&f, "foregroundSize")?,
                button_size: vec2(&f, "buttonSize")?,
                button_step: f32_of("buttonStep")?,
                marker_offset: vec2(&f, "selectionMarkerOffset")?,
                marker_anchor: [0.0, 0.0],
                height: 0.0,
                size_delta: [0.0, 0.0],
                parent_span_x: 0.0,
            },
            normal: sprite("backgroundSpriteNormal")?,
            selected: sprite("backgroundSpriteSelected")?,
            notification_anchor: f
                .get("notificationAnchor")
                .and_then(Value::as_i64)
                .ok_or_else(|| e("notificationAnchor: not read".into()))?
                as i32,
            notification_offset: vec2(&f, "notificationOffset")?,
        })
    }

    /// The PDA's screen prefab (`uGUI_PDAScreen`) as a UI tree, from the
    /// main scene's `PDA.prefabScreen`.
    pub fn pda_screen_tree(&self) -> Result<UiTree> {
        let main = self.scene("main")?;
        let (screen, _) = self.pda_screen_refs(&main)?;
        self.ui_tree("uGUI_PDAScreen", &screen)
    }

    /// `uGUI_PDA`'s nodes in `tree` (the screen prefab): its canvas group,
    /// content, toolbar group, back button, background, and each tab with
    /// what its `Open` / `Close` change (the regular tabs' `content`
    /// CanvasGroup; the intro's and the time capsule's own GameObject).
    pub fn pda_screen_config(&self, layouts: &Layouts, tree: &UiTree) -> Result<PdaScreenConfig> {
        let (root, c) = tree.scripts(TYPE).next().ok_or("no uGUI_PDA")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let e = |m: String| format!("{TYPE}.{m}");
        let node = |name: &str| -> Result<usize> {
            node_of(
                self,
                tree,
                &c.object,
                pptr(&f, name).map_err(e)?,
                &e(name.into()),
            )
        };
        let mut tabs = Vec::with_capacity(TAB_FIELDS.len());
        for (name, tab) in TAB_FIELDS {
            let p = pptr(&f, name).map_err(e)?;
            let at = node_of(self, tree, &c.object, p, &e(name.into()))?;
            let show = match tab {
                Tab::Intro | Tab::TimeCapsule => TabShow::Object(at),
                _ => {
                    let script = self
                        .resolve(&c.object.file, p)?
                        .ok_or_else(|| e(format!("{name}: not set")))?;
                    let (_, tf) = self.object_fields(layouts, &script)?;
                    let content = pptr(&tf, "content").map_err(|m| e(format!("{name}.{m}")))?;
                    TabShow::Content(node_of(
                        self,
                        tree,
                        &script,
                        content,
                        &e(format!("{name}.content")),
                    )?)
                }
            };
            tabs.push(TabNodes { tab, show });
        }
        // `uGUI_Toolbar`'s gamepad labels.
        let toolbar = self
            .resolve(&c.object.file, pptr(&f, "toolbar").map_err(e)?)?
            .ok_or_else(|| e("toolbar: not set".into()))?;
        let (_, tf) = self.object_fields(layouts, &toolbar)?;
        let mut cycle_texts = Vec::new();
        for name in ["cycleLeftBtnText", "cycleRightBtnText"] {
            let p = pptr(&tf, name).map_err(|m| e(format!("toolbar.{m}")))?;
            if !p.is_null() {
                cycle_texts.push(node_of(
                    self,
                    tree,
                    &toolbar,
                    p,
                    &e(format!("toolbar.{name}")),
                )?);
            }
        }
        Ok(PdaScreenConfig {
            root,
            cycle_texts,
            canvas_group: node("canvasGroup")?,
            content: node("content")?,
            toolbar_group: node("toolbarCanvasGroup")?,
            back_button: node("backButton")?,
            background: node("pdaBackground")?,
            tabs,
        })
    }
}
