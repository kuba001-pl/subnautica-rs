//! What `uGUI_CraftingMenu` reads (M9e4a; `docs/formats/ui.md` § The
//! crafting menu): its nodes on `uGUI/CraftingCanvas`, the craft tree from
//! the game's code as [`sn_ui::crafting::SourceNode`]s, the sprites
//! `SpriteManager` gives it, and the icons it makes by code.

use sn_ui::crafting::{CraftingMenuConfig, NodeAction, SourceNode};
use sn_ui::icon::NEW_RECT_SIZE;
use sn_ui::layout::RectTransform;
use sn_ui::material::UiMaterial;
use sn_unity::Value;

use crate::icons::foreground_material;
use crate::ui_scene::CodeNode;
use crate::{Assets, GameCode, Layouts, Result, Sprites, UiGraphicKind, UiScene, UiTree};

const MENU: &str = "uGUI_CraftingMenu";
/// `SpriteManager.Group.Background` and `Category`.
pub const GROUP_BACKGROUND: usize = 2;
pub const GROUP_CATEGORY: usize = 3;

/// The menu's nodes in the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CraftingMenuNodes {
    /// The script's own node.
    pub menu: usize,
    /// `iconsCanvas` (a `RectTransform`).
    pub icons_canvas: usize,
    /// `canvasGroup`'s node.
    pub canvas_group: usize,
    /// `canvasScaler`'s node (the canvas placed by it).
    pub canvas: usize,
}

impl Assets<'_> {
    /// `uGUI_CraftingMenu`'s `iconsCanvas`, `canvasGroup` and `canvasScaler`
    /// as tree nodes.
    pub fn crafting_menu_nodes(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
    ) -> Result<CraftingMenuNodes> {
        let (menu, c) = tree.scripts(MENU).next().ok_or("no uGUI_CraftingMenu")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let node_of = |field: &str| -> Result<usize> {
            let p = f
                .get(field)
                .and_then(Value::as_pptr)
                .filter(|p| !p.is_null())
                .ok_or_else(|| format!("{MENU}.{field}: not set"))?;
            let o = self
                .resolve(&c.object.file, p)?
                .ok_or_else(|| format!("{MENU}.{field}: not found"))?;
            tree.nodes
                .iter()
                .position(|n| n.components.iter().any(|k| k.object.key() == o.key()))
                .ok_or_else(|| format!("{MENU}.{field}: not in the tree"))
        };
        Ok(CraftingMenuNodes {
            menu,
            icons_canvas: node_of("iconsCanvas")?,
            canvas_group: node_of("canvasGroup")?,
            canvas: node_of("canvasScaler")?,
        })
    }
}

/// The craft tree `id` (`CraftTree.GetTree`) from the game's code.
pub fn craft_source(code: &GameCode, id: &str) -> Result<SourceNode> {
    let tree = code
        .craft_trees
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("no craft tree {id:?}"))?;
    let expand = code
        .tree_action("Expand")
        .ok_or("TreeAction.Expand: not read")?;
    let craft = code
        .tree_action("Craft")
        .ok_or("TreeAction.Craft: not read")?;
    fn convert(n: &sn_dotnet::CraftNode, expand: i32, craft: i32) -> SourceNode {
        SourceNode {
            id: n.id.clone(),
            action: if n.action == expand {
                NodeAction::Expand
            } else if n.action == craft {
                NodeAction::Craft
            } else {
                NodeAction::None
            },
            tech: n.tech_type,
            children: n
                .children
                .iter()
                .map(|c| convert(c, expand, craft))
                .collect(),
        }
    }
    Ok(convert(&tree.root, expand, craft))
}

/// The four background sprites (`SpriteManager.Get(Background, …)`, no
/// default): `categoryNormal`, `categoryHovered`, `craftNormal`,
/// `craftHovered`, as a config with the menu's nodes.
pub fn crafting_menu_config(
    nodes: &CraftingMenuNodes,
    sprites: &Sprites,
    scene: &mut UiScene,
) -> CraftingMenuConfig {
    let mut get = |name: &str| {
        sprites.get_no_default(GROUP_BACKGROUND, name).map(|s| {
            (
                scene.sprite_id(&s.object),
                [s.sprite.rect[2], s.sprite.rect[3]],
            )
        })
    };
    CraftingMenuConfig {
        icons_canvas: nodes.icons_canvas,
        canvas_group: nodes.canvas_group,
        expand_normal: get("categoryNormal"),
        expand_hovered: get("categoryHovered"),
        craft_normal: get("craftNormal"),
        craft_hovered: get("craftHovered"),
    }
}

/// A node's foreground sprite as `CreateIcon` asks for it: a category's
/// `SpriteManager.Get(Category, "<tree>_<id>")` (no default: `None` when
/// missing), a recipe's `SpriteManager.Get(techType)` (the default
/// `Unknown` when missing); its sprite name for the logs.
pub fn crafting_foreground<'s>(
    sprites: &'s Sprites,
    tree_id: &str,
    action: NodeAction,
    id: &str,
    tech_name: Option<&str>,
) -> Option<&'s crate::AtlasSprite> {
    match action {
        NodeAction::Expand => sprites.get_no_default(GROUP_CATEGORY, &format!("{tree_id}_{id}")),
        NodeAction::Craft => sprites.item_icon(tech_name.unwrap_or("")),
        NodeAction::None => sprites.default_sprite(),
    }
}

/// `CreateIcon`'s `new GameObject(id)` + `uGUI_ItemIcon` with `Init(anchor
/// and pivot (0.5, 0.5))` and its `Background` and `Foreground`
/// (`uGUI_Icon`s, each with its own copy of `iconMaterial`), as
/// [`crate::item_icon_nodes`] makes the inventory's.
pub fn crafting_icon_nodes(layer: u32, material: &UiMaterial, name: &str) -> Vec<CodeNode> {
    let rect = RectTransform {
        anchor_min: [0.5, 0.5],
        anchor_max: [0.5, 0.5],
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
        rect,
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
            name: name.into(),
            parent: None,
            rect,
            active_self: true,
            layer,
            graphic: None,
        },
        icon("Background"),
        icon("Foreground"),
    ]
}
