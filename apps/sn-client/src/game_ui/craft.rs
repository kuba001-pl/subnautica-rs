//! The crafting menu in the client (M9e4b, `docs/formats/ui.md` § The
//! crafting menu); the rules live in `sn_ui::crafting`. The main world
//! says when the menu is open (the fabricator's hand), sends the pointer,
//! the clicks and what the menu reads (known blueprints, the inventory's
//! counts, the craft's progress); this worker side draws the icons on
//! `CraftingCanvas`, hit-tests, shows the tooltip and sends back what a
//! click asks for.
use std::cell::RefCell;
use std::collections::BTreeSet;

use super::*;
use sn_ui::crafting::{Clicked, CraftingMenu, MenuWorld, Node, NodeAction, SourceNode, Unlock};
use sn_ui::draw::{apply, to_ancestor};
use sn_ui::item_icons::Sprite;
use sn_ui::tooltip::{IngredientFacts, RecipeFacts, craft_node, craft_recipe};

/// The craft tree `Fabricator.craftTree` names.
pub(crate) const TREE: &str = "Fabricator";

/// What the main world sends each frame.
#[derive(Clone, Debug, Default)]
pub(crate) struct CraftFrame {
    /// `uGUI_CraftingMenu` selected (opened by the hand, not yet closed).
    pub open: bool,
    pub script_sequence: u64,
    pub test_script: bool,
    /// `Crafter.OnCraftingBegin`'s `Lock` this frame.
    pub lock: bool,
    /// `client.inProgress` and `client.progress` (a request sent and not
    /// yet answered counts as in progress at 0).
    pub in_progress: bool,
    pub progress: f32,
    /// The player's known tech and inventory counts by tech.
    pub known: Vec<i32>,
    pub analyzed: Vec<i32>,
    pub counts: Vec<(i32, usize)>,
    /// The mouse (physical pixels from the bottom left) while open.
    pub pointer: Option<[f32; 2]>,
    /// `--menu`'s scripted pointer: the node path it rests on.
    pub scripted: Option<String>,
    /// Buttons pressed at the pointer this frame (0 left, 1 right).
    pub clicks: Vec<u8>,
}

/// What a click asked for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CraftOut {
    /// `PerformAction(techType)`.
    Craft(i32),
    /// A press on nothing: the menu deselected.
    Deselect,
    /// Test harness acknowledgement after applying this UI frame.
    ScriptAck(u64, bool, Vec<String>),
}

pub(super) struct CraftUi {
    pub menu: CraftingMenu,
    nodes: sn_assets::CraftingMenuNodes,
    src: SourceNode,
    template: sn_assets::UiScene,
    foregrounds: HashMap<String, Option<Sprite>>,
    names: HashMap<i32, String>,
    recipes: HashMap<i32, sn_sim::craft::Recipe>,
    hovered: Option<String>,
    /// `iconsCanvas`'s centre in the canvas's space and the canvas's
    /// layout, read at `Open`.
    laid: Option<([f32; 2], sn_ui::layout::Tree)>,
    known_seen: Vec<i32>,
    faded: bool,
    ingredients_logged: Option<String>,
    awaiting_request: Option<f32>,
}

/// The world the menu's rules read.
struct World {
    known: BTreeSet<i32>,
    counts: HashMap<i32, usize>,
}

impl CraftUi {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        assets: &sn_assets::Assets,
        layouts: &sn_assets::Layouts,
        tree: &sn_assets::UiTree,
        scene: &mut sn_assets::UiScene,
        sprites: &sn_assets::Sprites,
        material: &sn_ui::material::UiMaterial,
        code: &sn_assets::GameCode,
        data: &sn_assets::TechData,
    ) -> Result<Self, String> {
        let nodes = assets.crafting_menu_nodes(layouts, tree)?;
        let config = sn_assets::crafting_menu_config(&nodes, sprites, scene);
        let src = sn_assets::craft_source(code, TREE)?;
        let names: HashMap<i32, String> = code.tech_types.iter().cloned().collect();
        let mut foregrounds = HashMap::new();
        let mut recipes = HashMap::new();
        let mut stack = vec![(String::new(), &src)];
        while let Some((path, n)) = stack.pop() {
            let s = sn_assets::crafting_foreground(
                sprites,
                TREE,
                n.action,
                &n.id,
                names.get(&n.tech).map(String::as_str),
            );
            foregrounds.insert(
                path.clone(),
                s.map(|s| {
                    (
                        scene.sprite_id(&s.object),
                        [s.sprite.rect[2], s.sprite.rect[3]],
                    )
                }),
            );
            if n.action == NodeAction::Craft
                && let Some(r) = data
                    .get(n.tech)
                    .and_then(|e| sn_assets::craft_recipe(e, &code.tech_defaults).ok())
            {
                recipes.insert(n.tech, r);
            }
            for c in &n.children {
                let p = if path.is_empty() {
                    c.id.clone()
                } else {
                    format!("{path}/{}", c.id)
                };
                stack.push((p, c));
            }
        }
        let template = sn_assets::UiScene::from_code(
            TREE,
            sn_assets::crafting_icon_nodes(tree.nodes[nodes.icons_canvas].layer, material, TREE),
        )?;
        info!(
            "game UI crafting menu: {} on {} (iconsCanvas {}), tree {TREE}: {} recipes; backgrounds {:?}; tooltip ingredient icons (uGUI_TooltipIcon), pins, CraftTree notifications, FILL_RADIAL and the sounds not ported",
            tree.nodes[nodes.menu].path,
            tree.nodes[nodes.canvas].path,
            tree.nodes[nodes.icons_canvas].path,
            recipes.len(),
            [
                config.expand_normal,
                config.expand_hovered,
                config.craft_normal,
                config.craft_hovered
            ]
            .map(|s| s.is_some())
        );
        Ok(Self {
            menu: CraftingMenu::new(config, 0x5eed_c4af),
            nodes,
            src,
            template,
            foregrounds,
            names,
            recipes,
            hovered: None,
            laid: None,
            known_seen: Vec::new(),
            faded: false,
            ingredients_logged: None,
            awaiting_request: None,
        })
    }

    fn name(&self, tech: i32) -> String {
        self.names
            .get(&tech)
            .cloned()
            .unwrap_or_else(|| tech.to_string())
    }

    /// One frame of the menu; returns what the clicks asked for and
    /// whether the tooltip is shown.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn frame(
        &mut self,
        f: &CraftFrame,
        assets: &sn_assets::Assets,
        set: &sn_ui::text::FontSet,
        scene: &mut sn_assets::UiScene,
        language: &HashMap<String, String>,
        tooltips: &mut tooltip::Tooltips,
        start_state: &UiState,
        placements: &mut HashMap<usize, Placement>,
        rules: &mut UiState,
        camera: &UiCamera,
        screen: Option<[i32; 2]>,
        dt: f32,
        time: f32,
    ) -> Result<(Vec<CraftOut>, bool), String> {
        let mut out = Vec::new();
        let world = World {
            known: f.known.iter().copied().collect(),
            counts: f.counts.iter().copied().collect(),
        };
        let unlock = |t: i32| {
            if world.known.contains(&t) {
                Unlock::Available
            } else {
                Unlock::Hidden
            }
        };
        let has_recipe = |t: i32| self.recipes.contains_key(&t);
        let recipes = &self.recipes;
        let fulfilled = |t: i32| {
            recipes.get(&t).is_some_and(|r| {
                r.ingredients
                    .iter()
                    .all(|&(i, n)| world.counts.get(&i).copied().unwrap_or(0) >= n as usize)
            })
        };
        let w = MenuWorld {
            survival: true,
            blueprints: true,
            in_submarine: false,
            unlock: &unlock,
            has_recipe: &has_recipe,
            fulfilled: &fulfilled,
        };
        let failed = RefCell::new(None);
        let template = &self.template;
        let foregrounds = &self.foregrounds;
        let foreground = |n: &Node| foregrounds.get(&n.path).copied().flatten();
        let graft =
            |scene: &mut sn_assets::UiScene, parent: usize| match scene.graft(template, parent) {
                Ok(n) => IconNodes {
                    root: n,
                    background: n + 1,
                    foreground: n + 2,
                },
                Err(e) => {
                    *failed.borrow_mut() = Some(e);
                    IconNodes {
                        root: usize::MAX,
                        background: usize::MAX,
                        foreground: usize::MAX,
                    }
                }
            };
        if f.open && !self.menu.selected {
            let l = scene.layout_for(assets, set, placements, start_state)?;
            let r = l.rect(self.nodes.icons_canvas);
            let c = apply(
                &to_ancestor(&l, self.nodes.icons_canvas, self.nodes.canvas),
                [r.x + 0.5 * r.width, r.y + 0.5 * r.height, 0.0],
            );
            let made = self.menu.made;
            let opened = self.menu.open(
                &self.src,
                &w,
                f.in_progress,
                r.width,
                dt,
                &mut |p| graft(scene, p),
                &foreground,
            );
            info!(
                "game UI crafting menu at {time:.3} s: Open({TREE}) {}: iconsCanvas {}×{} centred at ({:.1}, {:.1}) on the canvas; {} nodes visible ({} known blueprints), top level {:?}; {} icons made by code{}",
                if opened { "ok" } else { "refused" },
                r.width,
                r.height,
                c[0],
                c[1],
                self.menu.nodes.len(),
                world.known.len(),
                self.menu
                    .placed()
                    .iter()
                    .map(|(p, a, s)| format!("{p} {s:.2} ({:.1}, {:.1})", a[0], a[1]))
                    .collect::<Vec<_>>(),
                self.menu.made - made,
                if f.in_progress {
                    "; locked (crafting)"
                } else {
                    ""
                }
            );
            self.laid = Some(([c[0], c[1]], l));
            self.known_seen = f.known.clone();
            self.faded = false;
        } else if !f.open && self.menu.selected {
            self.menu.close();
            self.hovered = None;
            self.laid = None;
            info!("game UI crafting menu at {time:.3} s: closed (OnDeselect), every icon hidden");
        }
        if self.menu.selected && f.known != self.known_seen {
            // `KnownTech.onChanged` → `Resync`.
            self.known_seen = f.known.clone();
            self.menu
                .resync(&self.src, &w, &mut |p| graft(scene, p), &foreground);
            info!(
                "game UI crafting menu at {time:.3} s: Resync (known tech changed): {} nodes visible",
                self.menu.nodes.len()
            );
        }
        if f.lock && self.menu.selected {
            self.menu.lock();
            info!("game UI crafting menu at {time:.3} s: Lock (the craft began): not interactable");
        }
        let mut tooltip_shown = false;
        if let (true, Some(size), Some((centre, layout))) = (self.menu.selected, screen, &self.laid)
        {
            let placed = *placements
                .get(&self.nodes.canvas)
                .ok_or("crafting menu: the canvas is not placed")?;
            let pixel = match &f.scripted {
                Some(path) => self
                    .menu
                    .placed()
                    .into_iter()
                    .find(|(p, _, _)| p == path)
                    .and_then(|(_, at, _)| {
                        let world = apply(
                            &placed.world_from_canvas(),
                            [centre[0] + at[0], centre[1] + at[1], 0.0],
                        );
                        camera.project(world, size)
                    }),
                None => f.pointer,
            };
            let point =
                pixel.and_then(|px| placed.ray_point(camera.position, camera.screen_ray(px, size)));
            let local = point.map(|p| [p[0] - centre[0], p[1] - centre[1]]);
            let over = local.and_then(|p| self.menu.hit(p));
            if over != self.hovered {
                info!(
                    "game UI crafting menu at {time:.3} s: pointer {pixel:?} → {local:?}: hovered {over:?}"
                );
                self.hovered = over.clone();
            }
            self.menu.hover(over.as_deref());
            for &button in &f.clicks {
                match over.as_deref() {
                    Some(path) => {
                        let c = self.menu.click(
                            path,
                            button,
                            &w,
                            dt,
                            &mut |p| graft(scene, p),
                            &foreground,
                        );
                        info!(
                            "game UI crafting menu at {time:.3} s: button {button} on {path}: {c:?}"
                        );
                        match c {
                            Some(Clicked::Craft(t)) => out.push(CraftOut::Craft(t)),
                            Some(Clicked::Pin(t)) => info!(
                                "game UI crafting menu: PinManager.TogglePin({}) not ported",
                                self.name(t)
                            ),
                            _ => {}
                        }
                    }
                    // `FPSInputModule`: a press on no handler deselects.
                    None if self.menu.interactable => {
                        info!(
                            "game UI crafting menu at {time:.3} s: button {button} on no icon: deselected"
                        );
                        out.push(CraftOut::Deselect);
                    }
                    None => {}
                }
            }
            // The tooltip (`uGUI_ItemIcon` is an `ITooltip`).
            if let (Some(path), Some(p)) = (over.as_deref(), point)
                && let Some(i) = self.menu.find(path)
            {
                let node = self.menu.nodes[i].clone();
                let lang = |k: &str| language.get(k).cloned().unwrap_or_else(|| k.into());
                let (mut prefix, postfix, icons) = match node.action {
                    NodeAction::Craft => {
                        let name = self.name(node.tech);
                        let r = self.recipes.get(&node.tech);
                        let facts = RecipeFacts {
                            name: lang(&name),
                            description: lang(&format!("Tooltip_{name}")),
                            amount: r.map_or(1, |r| r.amount as i32),
                            ingredients: r
                                .map(|r| {
                                    r.ingredients
                                        .iter()
                                        .map(|&(t, a)| {
                                            let tn = self.name(t);
                                            IngredientFacts {
                                                tech: t,
                                                name: language
                                                    .get(&format!("{tn}.TooltipIngredient"))
                                                    .filter(|v| !v.is_empty())
                                                    .cloned()
                                                    .unwrap_or_else(|| lang(&tn)),
                                                amount: a as i32,
                                                have: world.counts.get(&t).copied().unwrap_or(0)
                                                    as i32,
                                            }
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                            fulfilled: fulfilled(node.tech),
                            requires_ingredients: true,
                        };
                        craft_recipe(tooltips.strings(), &facts, !w.unlocked(node.tech))
                    }
                    _ => {
                        let (a, b) = craft_node(
                            tooltips.strings(),
                            &lang(&format!("{TREE}Menu_{}", node.id)),
                            self.menu.expanded[i],
                        );
                        (a, b, Vec::new())
                    }
                };
                // Until the separate tooltip icon row is ported, retain
                // its ingredient names, amounts and affordability colours
                // in the shared text block.
                for (_, text) in &icons {
                    prefix.push('\n');
                    prefix.push_str(text);
                }
                let graphic = self
                    .menu
                    .icons
                    .get(path)
                    .map_or(self.nodes.canvas, |i| i.nodes.root);
                let mut st = start_state.clone();
                st.apply(rules);
                st.apply(&self.menu.state);
                let id = path.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
                    (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
                });
                tooltips.show(
                    scene,
                    set,
                    path,
                    id,
                    prefix,
                    postfix,
                    p,
                    layout,
                    self.nodes.canvas,
                    graphic,
                    &placed,
                    camera,
                    &st,
                    placements,
                    rules,
                    time,
                )?;
                if self.ingredients_logged.as_deref() != Some(path) {
                    self.ingredients_logged = Some(path.to_string());
                    if !icons.is_empty() {
                        let texts: Vec<&str> = icons.iter().map(|(_, t)| t.as_str()).collect();
                        info!(
                            "game UI crafting menu: {path}'s tooltip ingredient icons (uGUI_TooltipIcon not ported, not drawn): {texts:?}"
                        );
                    }
                }
                tooltip_shown = true;
            }
        }
        let was_alpha = self.menu.alpha;
        // A click starts synchronously in the original client. Keep its
        // lock through this frame while our request travels to the server.
        if out.iter().any(|o| matches!(o, CraftOut::Craft(_))) {
            self.awaiting_request = Some(time);
        }
        // Frames already queued before the click still carry the old
        // main-world state. Bridge them until pending/job is observed.
        if f.in_progress || self.awaiting_request.is_some_and(|at| time - at > 2.0) {
            self.awaiting_request = None;
        }
        let in_progress = f.in_progress || self.awaiting_request.is_some();
        self.menu.update(&w, in_progress, f.progress, dt);
        if self.menu.alpha == 0.0 && was_alpha > 0.0 && !self.faded {
            self.faded = true;
            info!(
                "game UI crafting menu at {time:.3} s: faded out (alpha 0) at progress {:.3}",
                f.progress
            );
        }
        if self.menu.alpha == 1.0 && self.faded {
            self.faded = false;
            info!("game UI crafting menu at {time:.3} s: craft done: interactable again, alpha 1");
        }
        if let Some(e) = failed.borrow_mut().take() {
            return Err(format!("crafting menu: graft: {e}"));
        }
        rules.apply(&self.menu.state);
        if f.test_script {
            out.push(CraftOut::ScriptAck(
                f.script_sequence,
                self.menu.selected
                    && self.menu.interactable
                    && screen.is_some()
                    && self.laid.is_some(),
                self.menu
                    .placed()
                    .into_iter()
                    .filter_map(|(p, at, _)| {
                        (self.menu.hit(at).as_deref() == Some(p.as_str())).then_some(p)
                    })
                    .collect(),
            ));
        }
        Ok((out, tooltip_shown))
    }
}
