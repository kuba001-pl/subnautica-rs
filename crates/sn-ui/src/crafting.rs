//! `uGUI_CraftingMenu` (M9e4a; `docs/formats/ui.md` § The crafting menu):
//! the fabricator's menu of icons, our own code after reading the game's
//! `uGUI_CraftingMenu` and `uGUI_ItemIcon` (read only).
//!
//! - [`filter`] is `UpdateTree`: the craft tree as the player may see it.
//!   A category (`Expand`) needs a visible child and passes `Filter(id)`
//!   (`Survival` only in survival, `Submarine` only in a submarine); a
//!   recipe (`Craft`) with blueprints required needs ingredients and an
//!   unlock state of `Available` or `Locked`.
//! - [`icon_metrics`] is `GetIconMetrics`: one column when any sibling is a
//!   category, else a grid of ⌈√n⌉ columns; size max(40, 92 ÷ 1.18^(d−1))
//!   at depth d; x from the parent's right edge.
//! - [`CraftingMenu`]: `Open` (the tree, `Expand(root)`, locked while the
//!   machine crafts), `Expand` (siblings collapsed, the children's icons
//!   made or shown, each punched at amplitude 1 with a delay 0.1 · i ÷ n),
//!   `Collapse`, `Action` (expand, craft or deny; the icon punched at 0.5),
//!   the pointer's enter and exit (the background sprite), `UpdateVisuals`
//!   (chroma 1 only for what can be crafted now and only while not locked;
//!   while crafting the menu fades out over the first 30 % of the craft and
//!   the node and its parents show the progress), `Lock`, `Close`.
//! - The punch is `uGUI_ItemIcon.PunchScale`: a `CoroutineTween` over
//!   duration + delay on `PDA.deltaTime`; the foreground's and the
//!   background's scales are 1 + amplitude · `Oscillation(1 ÷ (0.01 ·
//!   duration), frequency, seed, t)`, t = (s − k) ÷ (1 − k), k = delay ÷
//!   (delay + duration).
//!
//! The game destroys the icons on close and makes new ones; we hide them
//! and keep their nodes for reuse (as [`crate::item_icons`]). The craft
//! tree's notification counts (`NotificationManager` group `CraftTree`)
//! and pinning (`PinManager`) are not ported. `UnityEngine.Random` is not
//! reproduced: our generator, the same ranges (as [`crate::notifier`]).

use std::collections::BTreeMap;

use crate::hud::Random;
use crate::item_icons::{IconNodes, Sprite};
use crate::state::UiState;
use crate::tween::{CoroutineTween, TweenMode, oscillation};

/// `TreeAction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeAction {
    None,
    Expand,
    Craft,
}

/// A craft tree's node as `CraftTree` stores it.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceNode {
    pub id: String,
    pub action: NodeAction,
    /// `techType0` (0 for categories).
    pub tech: i32,
    pub children: Vec<SourceNode>,
}

/// `TechUnlockState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unlock {
    Hidden,
    Locked,
    Available,
}

/// What the menu's rules read from the game each time they run.
pub struct MenuWorld<'a> {
    /// `GameModeUtils.RequiresSurvival()`.
    pub survival: bool,
    /// `GameModeUtils.RequiresBlueprints()`.
    pub blueprints: bool,
    /// `Player.main.IsInSubmarine()`.
    pub in_submarine: bool,
    /// `KnownTech.GetTechUnlockState`.
    pub unlock: &'a dyn Fn(i32) -> Unlock,
    /// `TechData.GetIngredients(techType) != null`.
    pub has_recipe: &'a dyn Fn(i32) -> bool,
    /// `CrafterLogic.IsCraftRecipeFulfilled`: every ingredient in the
    /// inventory (or none needed).
    pub fulfilled: &'a dyn Fn(i32) -> bool,
}

impl MenuWorld<'_> {
    /// `Filter(string id)`.
    fn filter_id(&self, id: &str) -> bool {
        (self.survival || id != "Survival") && (self.in_submarine || id != "Submarine")
    }

    /// `Filter(TechType)`.
    fn filter_tech(&self, tech: i32) -> bool {
        !self.blueprints
            || (self.has_recipe)(tech)
                && matches!((self.unlock)(tech), Unlock::Available | Unlock::Locked)
    }

    /// `CrafterLogic.IsCraftRecipeUnlocked`: known (or no blueprints).
    pub fn unlocked(&self, tech: i32) -> bool {
        !self.blueprints || (self.unlock)(tech) == Unlock::Available
    }
}

/// One node of the menu's tree (`uGUI_CraftingMenu.Node`).
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    /// The ids from the root's child down, joined by `/` (the root: "").
    pub path: String,
    pub action: NodeAction,
    pub tech: i32,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// `TreeNode.depth`: the root 0, its children 1.
    pub depth: usize,
}

/// `UpdateTree` on a new tree: the visible nodes, the root first and each
/// node's children in the tree's order. `None` when the root itself is
/// filtered out (a root category with nothing visible).
pub fn filter(src: &SourceNode, w: &MenuWorld) -> Option<Vec<Node>> {
    let mut out = Vec::new();
    keep(src, w, true, None, "", 0, &mut out).then_some(out)
}

/// Adds `src` (and what of it is visible) to `out` if it is visible.
fn keep(
    src: &SourceNode,
    w: &MenuWorld,
    mut available: bool,
    parent: Option<usize>,
    path: &str,
    depth: usize,
    out: &mut Vec<Node>,
) -> bool {
    if available && src.action == NodeAction::Expand {
        available = w.filter_id(&src.id);
    }
    let me = out.len();
    out.push(Node {
        id: src.id.clone(),
        path: path.to_string(),
        action: src.action,
        tech: src.tech,
        parent,
        children: Vec::new(),
        depth,
    });
    let mut children = Vec::new();
    for c in &src.children {
        let p = if path.is_empty() {
            c.id.clone()
        } else {
            format!("{path}/{}", c.id)
        };
        let at = out.len();
        if keep(c, w, available, Some(me), &p, depth + 1, out) {
            children.push(at);
        }
    }
    if available {
        available = match src.action {
            NodeAction::Expand => !children.is_empty(),
            NodeAction::Craft => w.filter_tech(src.tech),
            NodeAction::None => true,
        };
    }
    if available {
        out[me].children = children;
    } else {
        out.truncate(me);
    }
    available
}

/// `IsGrid`: no sibling is a category.
pub fn is_grid(nodes: &[Node], parent: usize) -> bool {
    nodes[parent]
        .children
        .iter()
        .all(|&c| nodes[c].action != NodeAction::Expand)
}

/// `GetIconMetrics`: (size, x, y) of the `index`th of `siblings` at
/// `depth`, under a parent `parent_width` wide (its rect).
pub fn icon_metrics(
    parent_width: f32,
    depth: usize,
    index: usize,
    siblings: usize,
    grid: bool,
) -> (f32, f32, f32) {
    let k = 1.0 / 1.18f32.powi(depth as i32 - 1);
    let size = (92.0 * k).max(40.0);
    let columns = if grid {
        (siblings as f32).sqrt().ceil() as usize
    } else {
        1
    }
    .max(1);
    let rows = (siblings.max(1) - 1) / columns + 1;
    let row = index / columns;
    let column = index - row * columns;
    let mut x = (column as f32 + 0.5) * size;
    let y = (0.5 * (rows as f32 - 1.0) - row as f32) * size;
    if parent_width > 0.0 {
        x += 0.5 * parent_width;
    }
    (size, x, y)
}

/// `MathExtensions.EaseInSine`.
pub fn ease_in_sine(v: f32) -> f32 {
    ((v - 1.0) * std::f32::consts::PI * 0.5).sin() + 1.0
}

/// The punch constants (`uGUI_CraftingMenu`).
pub const EXPAND_PUNCH_AMPLITUDE: f32 = 1.0;
pub const CLICK_PUNCH_AMPLITUDE: f32 = 0.5;
pub const PUNCH_DURATION: f32 = 1.0;
pub const PUNCH_DURATION_SCATTER: f32 = 0.2;
pub const CLICK_PUNCH_FREQUENCY: f32 = 5.0;
/// `CreateIcon`: the foreground is 0.674 of the icon.
pub const FOREGROUND_SCALE: f32 = 0.674;
/// While crafting, the menu fades out over this part of the craft
/// (`UpdateVisuals`: alpha 1 − progress ÷ 0.3).
pub const FADE_PART: f32 = 0.3;

/// An icon's running `PunchScale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Punch {
    pub tween: CoroutineTween,
    pub frequency: f32,
    pub amplitude: f32,
    pub duration: f32,
    pub delay: f32,
    pub seed: f32,
}

impl Punch {
    /// `OnPunchUpdate(scalar)`: the scale's x and y.
    pub fn scale(&self, scalar: f32) -> [f32; 2] {
        let reduction = 1.0 / (0.01 * self.duration);
        let k = self.delay / (self.delay + self.duration);
        let t = (scalar - k) / (1.0 - k);
        let (o, o1) = oscillation(reduction, self.frequency, self.seed, t);
        [1.0 + self.amplitude * o, 1.0 + self.amplitude * o1]
    }
}

/// One node's icon (`uGUI_ItemIcon`).
#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub nodes: IconNodes,
    /// Its own `SetActive`.
    pub active: bool,
    pub size: f32,
    /// `anchoredPosition` in its parent (the parent icon, or `iconsCanvas`),
    /// from the parent's centre.
    pub at: [f32; 2],
    pub hovered: bool,
    pub chroma: f32,
    pub progress: f32,
    pub punch: Option<Punch>,
    /// The punch's last scale (1, 1 at rest).
    pub scale: [f32; 2],
}

/// The menu's stored fields and sprites.
#[derive(Clone, Debug, PartialEq)]
pub struct CraftingMenuConfig {
    /// `iconsCanvas`'s node: the first icons' parent.
    pub icons_canvas: usize,
    /// `canvasGroup`'s node.
    pub canvas_group: usize,
    /// `SpriteManager.Background`'s `categoryNormal`, `categoryHovered`,
    /// `craftNormal`, `craftHovered` (`None`: not in the atlas).
    pub expand_normal: Option<Sprite>,
    pub expand_hovered: Option<Sprite>,
    pub craft_normal: Option<Sprite>,
    pub craft_hovered: Option<Sprite>,
}

/// What a click did (`Action`), for the caller and the logs.
#[derive(Clone, Debug, PartialEq)]
pub enum Clicked {
    /// A category opened (`soundExpand`).
    Expanded(String),
    /// `PerformAction(techType)`: the caller asks the machine to craft;
    /// `soundAccept`.
    Craft(i32),
    /// Unavailable, locked or not interactable: `soundDeny`.
    Denied(String),
    /// The right button on a recipe: `PinManager.TogglePin` (not ported).
    Pin(i32),
}

/// `uGUI_CraftingMenu`'s state.
#[derive(Clone, Debug)]
pub struct CraftingMenu {
    pub config: CraftingMenuConfig,
    /// `selected` (open).
    pub selected: bool,
    pub nodes: Vec<Node>,
    pub expanded: Vec<bool>,
    /// By node path.
    pub icons: BTreeMap<String, Icon>,
    pool: Vec<IconNodes>,
    pub interactable: bool,
    pub locked: bool,
    /// `canvasGroup.alpha`.
    pub alpha: f32,
    /// `craftedNode`'s path.
    pub crafted: Option<String>,
    dirty: bool,
    random: Random,
    /// Icons made by code since the menu was made.
    pub made: usize,
    /// The canvas width `iconsCanvas.rect.width` gave at `Open`.
    canvas_width: f32,
    /// The changes for the draw.
    pub state: UiState,
}

impl CraftingMenu {
    pub fn new(config: CraftingMenuConfig, seed: u32) -> CraftingMenu {
        CraftingMenu {
            config,
            selected: false,
            nodes: Vec::new(),
            expanded: Vec::new(),
            icons: BTreeMap::new(),
            pool: Vec::new(),
            interactable: true,
            locked: false,
            alpha: 1.0,
            crafted: None,
            dirty: false,
            random: Random::new(seed),
            made: 0,
            canvas_width: 0.0,
            state: UiState::default(),
        }
    }

    /// The node at `path`.
    pub fn find(&self, path: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.path == path)
    }

    /// `Open(treeType, receiver)`: false if already open or the tree is
    /// empty. `canvas_width` is `iconsCanvas.rect.width`; `make(parent)`
    /// makes an icon's nodes under a scene node; `foreground(node)` is the
    /// node's foreground sprite (`SpriteManager.Get`). `dt` is this frame's
    /// `PDA.deltaTime` (the punches' first step).
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &mut self,
        src: &SourceNode,
        w: &MenuWorld,
        in_progress: bool,
        canvas_width: f32,
        dt: f32,
        make: &mut dyn FnMut(usize) -> IconNodes,
        foreground: &dyn Fn(&Node) -> Option<Sprite>,
    ) -> bool {
        if self.selected || src.children.is_empty() {
            return false;
        }
        let Some(nodes) = filter(src, w) else {
            return false;
        };
        self.canvas_width = canvas_width;
        self.expanded = vec![false; nodes.len()];
        self.nodes = nodes;
        self.selected = true;
        self.alpha = 1.0;
        self.state
            .group_alpha
            .insert(self.config.canvas_group, self.alpha);
        self.expand(0, dt, make, foreground);
        if in_progress {
            self.set_locked(true);
        }
        true
    }

    /// `OnDeselect`: every icon gone, interactable again.
    pub fn close(&mut self) -> bool {
        if !self.selected {
            return false;
        }
        self.set_interactable(true);
        for (_, icon) in std::mem::take(&mut self.icons) {
            self.state.active.insert(icon.nodes.root, false);
            self.pool.push(icon.nodes);
        }
        self.nodes.clear();
        self.expanded.clear();
        self.crafted = None;
        self.selected = false;
        true
    }

    /// `Lock(receiver)`: not interactable (the craft has begun).
    pub fn lock(&mut self) {
        if self.selected {
            self.set_interactable(false);
        }
    }

    /// `Expand(node)`.
    fn expand(
        &mut self,
        node: usize,
        dt: f32,
        make: &mut dyn FnMut(usize) -> IconNodes,
        foreground: &dyn Fn(&Node) -> Option<Sprite>,
    ) {
        if let Some(p) = self.nodes[node].parent {
            for c in self.nodes[p].children.clone() {
                self.collapse(c);
            }
        }
        self.set_expanded(node, true);
        let grid = is_grid(&self.nodes, node);
        let children = self.nodes[node].children.clone();
        let n = children.len();
        let parent_width = self.parent_width(node);
        for (i, &c) in children.iter().enumerate() {
            let (size, x, y) = icon_metrics(parent_width, self.nodes[c].depth, i, n, grid);
            let path = self.nodes[c].path.clone();
            if !self.icons.contains_key(&path) {
                self.create_icon(c, size, [x, y], make, foreground);
            } else if let Some(icon) = self.icons.get_mut(&path) {
                icon.active = true;
                self.state.active.insert(icon.nodes.root, true);
            }
            self.punch(
                &path,
                0.1 * (i as f32 / n as f32),
                EXPAND_PUNCH_AMPLITUDE,
                dt,
            );
        }
    }

    /// `Collapse(parent)`: it and everything below it collapsed, the icons
    /// below it hidden.
    fn collapse(&mut self, node: usize) {
        self.set_expanded(node, false);
        let mut stack = self.nodes[node].children.clone();
        while let Some(n) = stack.pop() {
            self.set_expanded(n, false);
            if let Some(icon) = self.icons.get_mut(&self.nodes[n].path) {
                icon.active = false;
                self.state.active.insert(icon.nodes.root, false);
            }
            stack.extend(self.nodes[n].children.iter().copied());
        }
    }

    /// `GetRectTransform(node).rect.width`: the icon's size, or the canvas's
    /// for the root.
    fn parent_width(&self, node: usize) -> f32 {
        if self.nodes[node].parent.is_none() {
            return self.canvas_width;
        }
        self.icons
            .get(&self.nodes[node].path)
            .map_or(0.0, |i| i.size)
    }

    /// `CreateIcon`.
    fn create_icon(
        &mut self,
        node: usize,
        size: f32,
        at: [f32; 2],
        make: &mut dyn FnMut(usize) -> IconNodes,
        foreground: &dyn Fn(&Node) -> Option<Sprite>,
    ) {
        let Some(parent) = self.nodes[node].parent else {
            return;
        };
        let parent_scene = if self.nodes[parent].parent.is_none() {
            Some(self.config.icons_canvas)
        } else {
            self.icons
                .get(&self.nodes[parent].path)
                .map(|i| i.nodes.root)
        };
        let Some(parent_scene) = parent_scene else {
            return;
        };
        // A pooled icon is reused only under the same parent; the game makes
        // a new one each time.
        let nodes = make(parent_scene);
        self.made += 1;
        let background = match self.nodes[node].action {
            NodeAction::Expand => self.config.expand_normal,
            NodeAction::Craft => self.config.craft_normal,
            NodeAction::None => None,
        };
        let fg = foreground(&self.nodes[node]);
        let s = &mut self.state;
        let fg_size = FOREGROUND_SCALE * size;
        s.active.insert(nodes.root, true);
        s.transform.entry(nodes.root).or_default().size_delta = Some([size, size]);
        s.transform.entry(nodes.root).or_default().anchored_position = Some(at);
        // `SetBackgroundSize(size, size)`: `Fit` without the aspect;
        // `SetForegroundSize(f, f)`: `Fit` keeping it.
        set_sprite(s, nodes.background, background, [size, size], false);
        set_sprite(s, nodes.foreground, fg, [fg_size, fg_size], true);
        let fill = [
            -0.5 * size.max(fg_size),
            -0.5 * size.max(fg_size),
            size.max(fg_size),
            size.max(fg_size),
        ];
        let bg = s.material.entry(nodes.background).or_default();
        // An atlas sprite: no procedural slice 9 grid.
        bg.keywords.insert("SLICE_9_GRID".into(), false);
        bg.vectors.insert("_Size".into(), [size, size, 0.0, 0.0]);
        bg.vectors.insert("_FillRect".into(), fill);
        bg.floats.insert("_Radius".into(), size * 0.5);
        s.material
            .entry(nodes.foreground)
            .or_default()
            .vectors
            .insert("_FillRect".into(), fill);
        let path = self.nodes[node].path.clone();
        self.icons.insert(
            path,
            Icon {
                nodes,
                active: true,
                size,
                at,
                hovered: false,
                chroma: 1.0,
                progress: 0.0,
                punch: None,
                scale: [1.0, 1.0],
            },
        );
        self.dirty = true;
    }

    /// `Punch(icon, delay, amplitude)`.
    fn punch(&mut self, path: &str, delay: f32, amplitude: f32, dt: f32) {
        let duration = PUNCH_DURATION
            + self
                .random
                .range(-PUNCH_DURATION_SCATTER, PUNCH_DURATION_SCATTER);
        let seed = self.random.value();
        let Some(icon) = self.icons.get_mut(path) else {
            return;
        };
        let delay = delay.max(0.0);
        let mut tween = CoroutineTween::new(TweenMode::Once, duration + delay);
        let mut p = Punch {
            tween,
            frequency: CLICK_PUNCH_FREQUENCY,
            amplitude,
            duration,
            delay,
            seed,
        };
        let (_, calls) = tween.start(dt);
        p.tween = tween;
        icon.punch = Some(p);
        apply_punch(&mut self.state, icon, calls.update, calls.stopped);
    }

    fn set_expanded(&mut self, node: usize, expanded: bool) {
        if self.expanded[node] != expanded {
            self.expanded[node] = expanded;
            self.dirty = true;
        }
    }

    fn set_locked(&mut self, locked: bool) {
        if self.locked != locked {
            self.locked = locked;
            self.dirty = true;
        }
    }

    /// `SetInteractable`.
    fn set_interactable(&mut self, value: bool) {
        if self.interactable != value {
            self.interactable = value;
            if value {
                self.set_alpha(1.0);
            }
        }
    }

    /// `SetAlpha`: `EaseInSine` of the clamped value.
    fn set_alpha(&mut self, alpha: f32) {
        self.alpha = ease_in_sine(alpha.clamp(0.0, 1.0));
        self.state
            .group_alpha
            .insert(self.config.canvas_group, self.alpha);
    }

    /// `OnPointerEnter` / `OnPointerExit` of the icon at `path` (`None`:
    /// the pointer is on no icon).
    pub fn hover(&mut self, path: Option<&str>) {
        let paths: Vec<String> = self.icons.keys().cloned().collect();
        for p in paths {
            let on = path == Some(p.as_str());
            let Some(node) = self.find(&p) else { continue };
            let action = self.nodes[node].action;
            let Some(icon) = self.icons.get_mut(&p) else {
                continue;
            };
            if icon.hovered == on {
                continue;
            }
            icon.hovered = on;
            let sprite = match (action, on) {
                (NodeAction::Craft, false) => self.config.craft_normal,
                (NodeAction::Craft, true) => self.config.craft_hovered,
                (_, false) => self.config.expand_normal,
                (_, true) => self.config.expand_hovered,
            };
            let size = icon.size;
            set_sprite(
                &mut self.state,
                icon.nodes.background,
                sprite,
                [size, size],
                false,
            );
        }
    }

    /// `ActionAvailable`.
    pub fn action_available(&self, node: usize, w: &MenuWorld) -> bool {
        let n = &self.nodes[node];
        match n.action {
            NodeAction::Expand => true,
            NodeAction::Craft => w.unlocked(n.tech) && (w.fulfilled)(n.tech),
            NodeAction::None => false,
        }
    }

    /// `OnPointerClick(icon, button)` with the mouse (button 0 left, 1
    /// right). `None`: not on an icon, or ignored (not interactable).
    #[allow(clippy::too_many_arguments)]
    pub fn click(
        &mut self,
        path: &str,
        button: u8,
        w: &MenuWorld,
        dt: f32,
        make: &mut dyn FnMut(usize) -> IconNodes,
        foreground: &dyn Fn(&Node) -> Option<Sprite>,
    ) -> Option<Clicked> {
        if !self.interactable {
            return None;
        }
        let node = self.find(path)?;
        self.icons.get(path)?;
        match button {
            0 => Some(self.action(node, w, dt, make, foreground)),
            1 if self.nodes[node].action == NodeAction::Craft
                && w.unlocked(self.nodes[node].tech) =>
            {
                Some(Clicked::Pin(self.nodes[node].tech))
            }
            _ => None,
        }
    }

    /// `Action(sender)`. A craft is taken as begun (`PerformAction` true):
    /// the caller sends the request; if the machine refuses, the next
    /// [`update`](Self::update) with no craft in progress unlocks the menu.
    fn action(
        &mut self,
        node: usize,
        w: &MenuWorld,
        dt: f32,
        make: &mut dyn FnMut(usize) -> IconNodes,
        foreground: &dyn Fn(&Node) -> Option<Sprite>,
    ) -> Clicked {
        let path = self.nodes[node].path.clone();
        let result = if self.interactable && self.action_available(node, w) {
            match self.nodes[node].action {
                NodeAction::Expand => {
                    self.expand(node, dt, make, foreground);
                    Clicked::Expanded(path.clone())
                }
                _ => {
                    self.set_locked(true);
                    self.crafted = Some(path.clone());
                    Clicked::Craft(self.nodes[node].tech)
                }
            }
        } else {
            Clicked::Denied(path.clone())
        };
        self.punch(&path, 0.0, CLICK_PUNCH_AMPLITUDE, dt);
        result
    }

    /// `Resync` (the known tech changed): the tree filtered again. Icons of
    /// nodes still visible are kept; those gone are hidden; new children of
    /// expanded nodes get icons and every icon is placed again
    /// (`UpdateIcons`).
    pub fn resync(
        &mut self,
        src: &SourceNode,
        w: &MenuWorld,
        make: &mut dyn FnMut(usize) -> IconNodes,
        foreground: &dyn Fn(&Node) -> Option<Sprite>,
    ) {
        if !self.selected {
            return;
        }
        let Some(nodes) = filter(src, w) else {
            self.close();
            return;
        };
        let expanded: Vec<String> = self
            .nodes
            .iter()
            .zip(&self.expanded)
            .filter(|(_, e)| **e)
            .map(|(n, _)| n.path.clone())
            .collect();
        self.expanded = nodes.iter().map(|n| expanded.contains(&n.path)).collect();
        self.nodes = nodes;
        let gone: Vec<String> = self
            .icons
            .keys()
            .filter(|p| !self.nodes.iter().any(|n| &n.path == *p))
            .cloned()
            .collect();
        for p in gone {
            if let Some(icon) = self.icons.remove(&p) {
                self.state.active.insert(icon.nodes.root, false);
                self.pool.push(icon.nodes);
            }
        }
        self.update_icons(0, make, foreground);
        self.dirty = true;
    }

    /// `UpdateIcons(node)`.
    fn update_icons(
        &mut self,
        node: usize,
        make: &mut dyn FnMut(usize) -> IconNodes,
        foreground: &dyn Fn(&Node) -> Option<Sprite>,
    ) {
        let has_icon = self.icons.contains_key(&self.nodes[node].path);
        if !self.expanded[node] && !has_icon {
            return;
        }
        let grid = is_grid(&self.nodes, node);
        let children = self.nodes[node].children.clone();
        let n = children.len();
        for (i, &c) in children.iter().enumerate() {
            let parent_width = self.parent_width(node);
            let (size, x, y) = icon_metrics(parent_width, self.nodes[c].depth, i, n, grid);
            let path = self.nodes[c].path.clone();
            if let Some(icon) = self.icons.get_mut(&path) {
                icon.at = [x, y];
                self.state
                    .transform
                    .entry(icon.nodes.root)
                    .or_default()
                    .anchored_position = Some([x, y]);
            } else if self.expanded[node] {
                self.create_icon(c, size, [x, y], make, foreground);
            }
            self.update_icons(c, make, foreground);
        }
    }

    /// `UpdateVisuals` (each frame while open) and the punches' tweens
    /// (`PDA.deltaTime`). `progress` is `client.progress` while
    /// `in_progress`.
    pub fn update(&mut self, w: &MenuWorld, in_progress: bool, progress: f32, dt: f32) {
        for icon in self.icons.values_mut() {
            if let Some(mut p) = icon.punch {
                let calls = if icon.active {
                    p.tween.move_next(dt)
                } else {
                    // `OnDisable`: `punchTween.Stop()`.
                    p.tween.stop();
                    crate::tween::TweenCalls {
                        update: None,
                        stopped: true,
                    }
                };
                icon.punch = (!calls.stopped).then_some(p);
                apply_punch(&mut self.state, icon, calls.update, calls.stopped);
            }
        }
        if !self.selected {
            return;
        }
        if self.dirty {
            self.dirty = false;
            self.update_notifications(0, w);
        }
        if !self.locked {
            return;
        }
        if in_progress {
            if !self.interactable {
                self.set_alpha(1.0 - progress / FADE_PART);
            }
            self.set_progress(self.crafted.clone(), progress);
        } else {
            self.set_interactable(true);
            self.set_progress(self.crafted.clone(), 1.0);
            self.crafted = None;
            self.set_locked(false);
            // `SetLocked` marks the chroma dirty; `UpdateVisuals` handles it
            // on the next frame.
        }
    }

    /// `UpdateNotifications`: returns the craftable count below `node`
    /// (chroma per icon; the notification counts are not ported).
    fn update_notifications(&mut self, node: usize, w: &MenuWorld) -> usize {
        let mut below = 0;
        for c in self.nodes[node].children.clone() {
            below += self.update_notifications(c, w);
        }
        let mut craftables = below;
        let available = match self.nodes[node].action {
            NodeAction::Craft => {
                let a = self.action_available(node, w);
                craftables += usize::from(a);
                a
            }
            NodeAction::Expand => below > 0,
            NodeAction::None => false,
        };
        let parent_expanded = self.nodes[node].parent.is_some_and(|p| self.expanded[p]);
        let chroma = if available && !self.locked { 1.0 } else { 0.0 };
        if let Some(icon) = self.icons.get_mut(&self.nodes[node].path)
            && parent_expanded
        {
            icon.chroma = chroma;
            for n in [icon.nodes.foreground, icon.nodes.background] {
                let m = self.state.material.entry(n).or_default();
                m.keywords.insert("GRAYSCALE".into(), chroma < 1.0);
                m.floats.insert("_Chroma".into(), chroma);
            }
        }
        craftables
    }

    /// `SetProgress(node, progress)`: the node's icon and its parents'.
    fn set_progress(&mut self, path: Option<String>, progress: f32) {
        let mut node = path.and_then(|p| self.find(&p));
        while let Some(n) = node {
            if let Some(icon) = self.icons.get_mut(&self.nodes[n].path) {
                let p = progress.clamp(0.0, 1.0);
                icon.progress = p;
                for g in [icon.nodes.foreground, icon.nodes.background] {
                    let m = self.state.material.entry(g).or_default();
                    m.keywords.insert("FILL_RADIAL".into(), p < 1.0);
                    m.floats.insert("_FillValue".into(), p);
                }
            }
            node = self.nodes[n].parent;
        }
    }

    /// Each shown icon's centre in `iconsCanvas`'s space (from its rect's
    /// centre) and its size, parents first.
    pub fn placed(&self) -> Vec<(String, [f32; 2], f32)> {
        let mut out = Vec::new();
        for n in &self.nodes {
            let Some(icon) = self.icons.get(&n.path) else {
                continue;
            };
            let mut at = icon.at;
            let mut shown = icon.active;
            let mut p = n.parent;
            while let Some(q) = p {
                if let Some(pi) = self.icons.get(&self.nodes[q].path) {
                    at[0] += pi.at[0];
                    at[1] += pi.at[1];
                    shown &= pi.active;
                }
                p = self.nodes[q].parent;
            }
            if shown {
                out.push((n.path.clone(), at, icon.size));
            }
        }
        out
    }

    /// The graphic raycast at `p` (`iconsCanvas`'s space): the icon whose
    /// rect holds it, the last drawn (deepest, latest) first. Not while the
    /// canvas group blocks no raycasts (`SetInteractable(false)`).
    pub fn hit(&self, p: [f32; 2]) -> Option<String> {
        if !self.selected || !self.interactable {
            return None;
        }
        self.placed()
            .into_iter()
            .rev()
            .find(|(_, at, size)| {
                (p[0] - at[0]).abs() <= 0.5 * size && (p[1] - at[1]).abs() <= 0.5 * size
            })
            .map(|(path, _, _)| path)
    }
}

/// `SetBackgroundSprite` / `SetForegroundSprite` with their `Fit`.
fn set_sprite(
    state: &mut UiState,
    node: usize,
    sprite: Option<Sprite>,
    size: [f32; 2],
    aspect: bool,
) {
    match sprite {
        Some((id, sprite_size)) => {
            state.sprite.insert(node, Some(id));
            state.enabled.insert(node, true);
            state.transform.entry(node).or_default().size_delta =
                Some(crate::icon::fit(size, sprite_size, aspect));
        }
        None => {
            state.sprite.insert(node, None);
            state.enabled.insert(node, false);
        }
    }
}

/// `OnPunchUpdate` / `OnPunchStop` on the graphics' scales.
fn apply_punch(state: &mut UiState, icon: &mut Icon, update: Option<f32>, stopped: bool) {
    let scale = match (stopped, update, icon.punch) {
        (true, _, _) => Some([1.0, 1.0]),
        (false, Some(s), Some(p)) => Some(p.scale(s)),
        _ => None,
    };
    if let Some(s) = scale {
        icon.scale = s;
        for n in [icon.nodes.foreground, icon.nodes.background] {
            state.transform.entry(n).or_default().scale = Some([s[0], s[1], 1.0]);
        }
    }
}

#[cfg(test)]
mod tests;
