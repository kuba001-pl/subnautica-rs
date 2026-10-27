//! `ui --crafting` (M9e4a, `docs/formats/ui.md` § The crafting menu):
//! `uGUI_CraftingMenu` read from the game (its nodes, the four background
//! sprites, the Fabricator tree from the code), then its rules
//! (`sn_ui::crafting`) checked against a replay written here apart from
//! them, in doubles, from the decompiled `UpdateTree`, `Filter`,
//! `GetIconMetrics` and `UpdateNotifications`:
//! - the node count with every recipe known (= P1's 100) and with the
//!   starting blueprints (`PDAData.defaultTech`);
//! - the menu opened with the starting blueprints and one Metal salvage:
//!   every top-level icon's size and centre; then the first category
//!   expanded, then its first category: each child's size and centre,
//!   both from the rules and from our layout of the grafted icons (the
//!   icon's rect centre in `iconsCanvas`'s space), N/N;
//! - each shown icon's chroma (1 only for what can be crafted now, and a
//!   category holding such a recipe);
//! - the recipe and category tooltips' texts (`TooltipFactory.CraftRecipe`
//!   and `CraftNode`) for the craftable recipe;
//! - a click on that recipe: `PerformAction` of its tech, the menu locked;
//!   the draw: two draws per shown icon (background and foreground), the
//!   grey ones with `GRAYSCALE`.

use std::collections::{BTreeSet, HashMap};
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, UiScene, UiState};
use sn_install::GameData;
use sn_ui::crafting::{
    Clicked, CraftingMenu, MenuWorld, Node, NodeAction, SourceNode, Unlock, filter,
};
use sn_ui::draw::{apply, to_ancestor};
use sn_ui::item_icons::IconNodes;
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};
use sn_ui::tooltip::{IngredientFacts, RecipeFacts, TooltipStrings, craft_node, craft_recipe};

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
const DT: f32 = 0.0173;
const TREE: &str = "Fabricator";
/// The inventory: one of this.
const HELD: &str = "ScrapMetal";

/// The replay's node: path, action, tech, children.
struct R {
    path: String,
    action: NodeAction,
    tech: i32,
    children: Vec<R>,
}

/// `UpdateTree` from the decompiled code: `None` when filtered out.
fn replay_tree(
    src: &SourceNode,
    known: &dyn Fn(i32) -> bool,
    path: &str,
    avail: bool,
) -> Option<R> {
    let mut available = avail;
    if available && src.action == NodeAction::Expand {
        // `Filter(id)`: survival (true), not in a submarine.
        available = src.id != "Submarine";
    }
    let mut children = Vec::new();
    for c in &src.children {
        let p = if path.is_empty() {
            c.id.clone()
        } else {
            format!("{path}/{}", c.id)
        };
        if let Some(r) = replay_tree(c, known, &p, available) {
            children.push(r);
        }
    }
    if available {
        available = match src.action {
            NodeAction::Expand => !children.is_empty(),
            // `Filter(TechType)` with blueprints: known (no partial
            // scanner or compound unlocks in our run).
            NodeAction::Craft => known(src.tech),
            NodeAction::None => true,
        };
    }
    available.then(|| R {
        path: path.to_string(),
        action: src.action,
        tech: src.tech,
        children,
    })
}

fn count(r: &R) -> usize {
    1 + r.children.iter().map(count).sum::<usize>()
}

/// `GetIconMetrics` in doubles: (size, x, y).
fn replay_metrics(
    parent_w: f64,
    depth: i32,
    index: usize,
    n: usize,
    grid: bool,
) -> (f64, f64, f64) {
    let size = (92.0 / 1.18f64.powi(depth - 1)).max(40.0);
    let cols = if grid {
        (n as f64).sqrt().ceil() as usize
    } else {
        1
    };
    let rows = (n - 1) / cols + 1;
    let row = index / cols;
    let col = index - row * cols;
    let mut x = (col as f64 + 0.5) * size;
    let y = (0.5 * (rows as f64 - 1.0) - row as f64) * size;
    if parent_w > 0.0 {
        x += 0.5 * parent_w;
    }
    (size, x, y)
}

/// The children of `r` (at `depth` + 1) placed about `origin` (the parent's
/// centre in `iconsCanvas`'s space): path → (size, centre).
fn replay_children(
    r: &R,
    depth: i32,
    parent_w: f64,
    origin: [f64; 2],
) -> Vec<(String, f64, [f64; 2])> {
    let grid = r.children.iter().all(|c| c.action != NodeAction::Expand);
    let n = r.children.len();
    r.children
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let (s, x, y) = replay_metrics(parent_w, depth + 1, i, n, grid);
            (c.path.clone(), s, [origin[0] + x, origin[1] + y])
        })
        .collect()
}

fn find<'r>(r: &'r R, path: &str) -> Option<&'r R> {
    if r.path == path {
        return Some(r);
    }
    r.children.iter().find_map(|c| find(c, path))
}

fn walk(src: &SourceNode) -> usize {
    1 + src.children.iter().map(walk).sum::<usize>()
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let tags = sn_assets::physics_settings(&assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let catalog = assets.catalog()?;
    let sprites = assets.sprites(&catalog)?;
    let root = assets.ugui_root(&catalog)?;
    let tree = assets.ui_tree("uGUI.prefab", &root)?;
    let start = UiState::from(&assets.ui_start_state(&layouts, &tree)?);
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let camera = UiCamera::game(DEFAULT_FIELD_OF_VIEW, NEAR, FAR);
    let mut placements = HashMap::new();
    for (node, s) in assets.ui_scalers(&tree)? {
        if let Ok(p) = place(&s, &camera, DEFAULT_FIELD_OF_VIEW, SCREEN, DEFAULT_UI_SCALE) {
            placements.insert(node, p);
        }
    }
    let (mut ok, mut all) = (0usize, 0usize);
    let mut check = |good: bool, what: &str| {
        all += 1;
        if good {
            ok += 1;
        } else {
            println!("  FAIL: {what}");
        }
    };

    let nodes = assets.crafting_menu_nodes(&layouts, &tree)?;
    let l0 = scene.layout_for(&assets, &set, &placements, &start)?;
    let canvas_rect = l0.rect(nodes.icons_canvas);
    println!(
        "uGUI_CraftingMenu on {}: iconsCanvas {} {canvas_rect:?}, canvasGroup {}, canvas {} (active at start {})",
        tree.nodes[nodes.menu].path,
        tree.nodes[nodes.icons_canvas].path,
        tree.nodes[nodes.canvas_group].path,
        tree.nodes[nodes.canvas].path,
        l0.active(nodes.icons_canvas)
    );
    let config = sn_assets::crafting_menu_config(&nodes, &sprites, &mut scene);
    println!(
        "  backgrounds: categoryNormal {:?}, categoryHovered {:?}, craftNormal {:?}, craftHovered {:?}",
        config.expand_normal.map(|s| s.1),
        config.expand_hovered.map(|s| s.1),
        config.craft_normal.map(|s| s.1),
        config.craft_hovered.map(|s| s.1)
    );
    check(
        [
            config.expand_normal,
            config.expand_hovered,
            config.craft_normal,
            config.craft_hovered,
        ]
        .iter()
        .all(Option::is_some),
        "the four background sprites are in the Backgrounds atlas",
    );

    // The tree, the blueprints and the recipes.
    let code = sn_assets::game_code(&sn_assets::read_assembly(game)?)?;
    let src = sn_assets::craft_source(&code, TREE)?;
    let p1 = walk(&src);
    let player = sn_assets::player_data(&assets)?;
    let known: BTreeSet<i32> = player.pda.default_tech.iter().copied().collect();
    let data = sn_assets::tech_data(&assets)?;
    let held = code
        .tech_types
        .iter()
        .find(|(_, n)| n == HELD)
        .map(|(t, _)| *t)
        .ok_or("ScrapMetal: no tech type")?;
    let recipe = |t: i32| {
        data.get(t)
            .and_then(|e| sn_assets::craft_recipe(e, &code.tech_defaults).ok())
    };
    let fulfilled = |t: i32| {
        recipe(t).is_some_and(|r| {
            r.ingredients
                .iter()
                .all(|&(i, n)| (i == held && n <= 1) || n == 0)
        })
    };
    let has_recipe = |t: i32| data.get(t).is_some();
    let everything = |_: i32| Unlock::Available;
    let by_known = |t: i32| {
        if known.contains(&t) {
            Unlock::Available
        } else {
            Unlock::Hidden
        }
    };
    let world_all = MenuWorld {
        survival: true,
        blueprints: true,
        in_submarine: false,
        unlock: &everything,
        has_recipe: &has_recipe,
        fulfilled: &fulfilled,
    };
    let w = MenuWorld {
        unlock: &by_known,
        ..world_all
    };
    let all_known = filter(&src, &world_all).map_or(0, |n| n.len());
    let replay_all = replay_tree(&src, &|t| has_recipe(t), "", true).map_or(0, |r| count(&r));
    println!(
        "tree {TREE}: {p1} nodes (P1); visible with every recipe known {all_known} (replay {replay_all}); {} starting blueprints",
        known.len()
    );
    check(p1 == 100, "the Fabricator tree has P1's 100 nodes");
    check(
        all_known == p1 && replay_all == p1,
        "every node visible when everything is known",
    );
    let r = replay_tree(&src, &|t| known.contains(&t) && has_recipe(t), "", true)
        .ok_or("replay: nothing visible")?;
    let visible = filter(&src, &w).ok_or("nothing visible")?;
    println!(
        "  with the starting blueprints: {} visible (replay {})",
        visible.len(),
        count(&r)
    );
    check(visible.len() == count(&r), "visible node count = replay");
    let same_paths = visible
        .iter()
        .map(|n| n.path.as_str())
        .collect::<BTreeSet<_>>()
        == {
            let mut s = BTreeSet::new();
            fn add<'a>(r: &'a R, s: &mut BTreeSet<&'a str>) {
                s.insert(r.path.as_str());
                for c in &r.children {
                    add(c, s);
                }
            }
            add(&r, &mut s);
            s
        };
    check(same_paths, "the same visible paths as the replay");

    // The menu, opened.
    let material = assets.icon_material(&catalog, &layouts)?;
    let layer = tree.nodes[nodes.icons_canvas].layer;
    let template =
        UiScene::from_code(TREE, sn_assets::crafting_icon_nodes(layer, &material, TREE))?;
    let names: HashMap<i32, String> = code.tech_types.iter().cloned().collect();
    let mut fg_ids: HashMap<String, Option<(sn_assets::SpriteId, [f32; 2])>> = HashMap::new();
    for n in &visible {
        let s = sn_assets::crafting_foreground(
            &sprites,
            TREE,
            n.action,
            &n.id,
            names.get(&n.tech).map(String::as_str),
        );
        fg_ids.insert(
            n.path.clone(),
            s.map(|s| {
                (
                    scene.sprite_id(&s.object),
                    [s.sprite.rect[2], s.sprite.rect[3]],
                )
            }),
        );
    }
    let missing_fg: Vec<&String> = fg_ids
        .iter()
        .filter(|(p, s)| !p.is_empty() && s.is_none())
        .map(|(p, _)| p)
        .collect();
    println!(
        "  foreground sprites: {} found, missing {missing_fg:?}",
        fg_ids.len() - 1 - missing_fg.len()
    );
    let foreground = |n: &Node| fg_ids.get(&n.path).copied().flatten();
    let mut menu = CraftingMenu::new(config.clone(), 0x5eed_c4af);
    let failed = std::cell::RefCell::new(None);
    let make = |scene: &mut UiScene, parent: usize| match scene.graft(&template, parent) {
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
    let opened = menu.open(
        &src,
        &w,
        false,
        canvas_rect.width,
        DT,
        &mut |p| make(&mut scene, p),
        &foreground,
    );
    check(opened, "Open");

    // Each step: the rules' and our layout's centres against the replay.
    let compare = |menu: &CraftingMenu,
                   scene: &mut UiScene,
                   expected: &[(String, f64, [f64; 2])],
                   what: &str,
                   check: &mut dyn FnMut(bool, &str)|
     -> Result<()> {
        let mut st = start.clone();
        st.apply(&menu.state);
        let l = scene.layout_for(&assets, &set, &placements, &st)?;
        let c = l.rect(nodes.icons_canvas);
        let centre = [c.x + 0.5 * c.width, c.y + 0.5 * c.height];
        let placed = menu.placed();
        let (mut good, mut worst) = (0, 0.0f64);
        for (path, size, at) in expected {
            let rule = placed.iter().find(|(p, _, _)| p == path);
            let icon = menu.icons.get(path);
            let laid = icon.map(|i| {
                let r = l.rect(i.nodes.root);
                let p = apply(
                    &to_ancestor(&l, i.nodes.root, nodes.icons_canvas),
                    [r.x + 0.5 * r.width, r.y + 0.5 * r.height, 0.0],
                );
                ([p[0] - centre[0], p[1] - centre[1]], r.width)
            });
            let d = match (rule, laid) {
                (Some((_, a, s)), Some((b, w))) => [
                    (f64::from(a[0]) - at[0]).abs(),
                    (f64::from(a[1]) - at[1]).abs(),
                    (f64::from(b[0]) - at[0]).abs(),
                    (f64::from(b[1]) - at[1]).abs(),
                    (f64::from(*s) - size).abs(),
                    (f64::from(w) - size).abs(),
                ]
                .into_iter()
                .fold(0.0, f64::max),
                _ => f64::INFINITY,
            };
            worst = worst.max(d);
            if d < 1e-3 {
                good += 1;
            } else {
                println!("    {path}: rule {rule:?}, layout {laid:?}, replay {size} {at:?}");
            }
        }
        let shown = placed.len();
        println!(
            "  {what}: {good}/{} icons at the replay's size and centre (worst {worst:.2e}); {shown} shown",
            expected.len()
        );
        check(good == expected.len() && !expected.is_empty(), what);
        Ok(())
    };
    let top = replay_children(&r, 0, f64::from(canvas_rect.width), [0.0, 0.0]);
    println!(
        "  top level: {:?}",
        top.iter()
            .map(|(p, s, a)| format!("{p} {s:.2} ({:.1}, {:.1})", a[0], a[1]))
            .collect::<Vec<_>>()
    );
    compare(&menu, &mut scene, &top, "top level", &mut check)?;
    check(menu.placed().len() == top.len(), "only the top level shown");

    // The first category, then its first category.
    let first = r
        .children
        .iter()
        .find(|c| c.action == NodeAction::Expand)
        .ok_or("no category")?;
    let mut chain = vec![first];
    if let Some(sub) = first
        .children
        .iter()
        .find(|c| c.action == NodeAction::Expand)
    {
        chain.push(sub);
    }
    let mut expected_shown = top.len();
    for (depth, node) in chain.iter().enumerate() {
        let (parent_size, origin) = {
            let placed = menu.placed();
            let p = placed.iter().find(|(p, _, _)| *p == node.path).cloned();
            p.map(|(_, a, s)| (f64::from(s), [f64::from(a[0]), f64::from(a[1])]))
                .ok_or_else(|| format!("{}: not shown", node.path))?
        };
        let clicked = menu.click(
            &node.path,
            0,
            &w,
            DT,
            &mut |p| make(&mut scene, p),
            &foreground,
        );
        check(
            clicked == Some(Clicked::Expanded(node.path.clone())),
            "a category click expands it",
        );
        let kids = replay_children(node, depth as i32 + 1, parent_size, origin);
        // Siblings' children collapse; this level adds its children.
        expected_shown += kids.len();
        println!(
            "  {} expanded: {:?}",
            node.path,
            kids.iter()
                .map(|(p, s, a)| format!("{p} {s:.2} ({:.1}, {:.1})", a[0], a[1]))
                .collect::<Vec<_>>()
        );
        compare(
            &menu,
            &mut scene,
            &kids,
            &format!("{} children", node.path),
            &mut check,
        )?;
        check(
            menu.placed().len() == expected_shown,
            "shown = the open levels' icons",
        );
    }
    if let Some(e) = failed.borrow_mut().take() {
        return Err(format!("graft: {e}"));
    }

    // The chroma.
    menu.update(&w, false, 0.0, DT);
    let craftable = |n: &R| -> bool {
        fn any(n: &R, ok: &dyn Fn(i32) -> bool) -> bool {
            match n.action {
                NodeAction::Craft => ok(n.tech),
                _ => n.children.iter().any(|c| any(c, ok)),
            }
        }
        any(n, &|t| known.contains(&t) && fulfilled(t))
    };
    let (mut good, mut shown, mut grey) = (0, 0, 0);
    let mut craft_paths = Vec::new();
    for (path, _, _) in menu.placed() {
        let Some(n) = find(&r, &path) else { continue };
        shown += 1;
        let want = if craftable(n) { 1.0 } else { 0.0 };
        if n.action == NodeAction::Craft && want == 1.0 {
            craft_paths.push(path.clone());
        }
        grey += usize::from(want == 0.0);
        if menu.icons[&path].chroma == want {
            good += 1;
        } else {
            println!(
                "    {path}: chroma {} (replay {want})",
                menu.icons[&path].chroma
            );
        }
    }
    println!(
        "  chroma: {good}/{shown} as the replay ({grey} grey); craftable shown {craft_paths:?}"
    );
    check(good == shown && shown > 0, "chroma = the replay");

    // The tooltips.
    let language = sn_assets::language(game, "English")?;
    let lang = |k: &str| language.get(k).cloned().unwrap_or_else(|| k.to_string());
    let mut buttons: [String; 4] = Default::default();
    for (b, name) in ["UISubmit", "UICancel", "UIClear", "UIAssign"]
        .iter()
        .enumerate()
    {
        buttons[b] = sn_assets::format_button(game, &set, &language, name)?.text;
    }
    let strings = TooltipStrings::new(&|k| lang(k), buttons);
    let category = &chain[chain.len() - 1].path;
    let id = category.rsplit('/').next().unwrap_or(category);
    let (prefix, postfix) = craft_node(&strings, &lang(&format!("{TREE}Menu_{id}")), true);
    println!("  tooltip of {category} (expanded): {prefix:?} / {postfix:?}");
    check(
        !prefix.contains(&format!("{TREE}Menu_")),
        "the category's label is in the language file",
    );
    if let Some(path) = craft_paths.first() {
        let n = find(&r, path).ok_or("craftable: not in the replay")?;
        let name = names.get(&n.tech).cloned().unwrap_or_default();
        let rec = recipe(n.tech).ok_or("craftable: no recipe")?;
        let facts = RecipeFacts {
            name: lang(&name),
            description: lang(&format!("Tooltip_{name}")),
            amount: rec.amount as i32,
            ingredients: rec
                .ingredients
                .iter()
                .map(|&(t, a)| {
                    let tn = names.get(&t).cloned().unwrap_or_default();
                    IngredientFacts {
                        tech: t,
                        // `GetOrFallback`: an empty value falls back too.
                        name: language
                            .get(&format!("{tn}.TooltipIngredient"))
                            .filter(|v| !v.is_empty())
                            .cloned()
                            .unwrap_or_else(|| lang(&tn)),
                        amount: a as i32,
                        have: i32::from(t == held),
                    }
                })
                .collect(),
            fulfilled: true,
            requires_ingredients: true,
        };
        let (prefix, postfix, icons) = craft_recipe(&strings, &facts, false);
        println!("  tooltip of {path}: prefix {prefix:?}; icons {icons:?}; postfix {postfix:?}");
        check(
            prefix.contains(&lang(&name))
                && !icons.is_empty()
                && icons.iter().all(|(t, i)| {
                    i.contains(&lang(names.get(t).map_or("", String::as_str)))
                        && !i.contains("></color>")
                })
                && postfix.contains(&lang("Craft")),
            "the recipe tooltip names it, lists its ingredients and offers the craft",
        );
        // The click.
        let c = menu.click(path, 0, &w, DT, &mut |p| make(&mut scene, p), &foreground);
        println!("  click on {path}: {c:?}; locked {}", menu.locked);
        check(
            c == Some(Clicked::Craft(n.tech)) && menu.locked,
            "a click crafts it and locks the menu",
        );
        menu.update(&w, true, 0.0, DT);
        menu.lock();
        menu.update(&w, true, 0.15, DT);
        println!("  craft at 15 %: canvas group alpha {:.4}", menu.alpha);
        check(
            (menu.alpha - sn_ui::crafting::ease_in_sine(0.5)).abs() < 1e-6,
            "the fade",
        );
        menu.update(&w, false, 0.0, DT);
        menu.update(&w, false, 0.0, DT);
        check(
            menu.interactable && !menu.locked && menu.alpha == 1.0,
            "done: open again",
        );
    } else {
        check(false, "a craftable recipe is shown");
    }

    // The draw.
    let mut st = start.clone();
    st.apply(&menu.state);
    let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &st)?;
    let placed = menu.placed();
    let (mut two, mut grey_drawn, mut grey_want) = (0, 0, 0);
    for (path, _, _) in &placed {
        let i = &menu.icons[path];
        let items: Vec<_> = d
            .list
            .items
            .iter()
            .filter(|it| it.node == i.nodes.background || it.node == i.nodes.foreground)
            .collect();
        let has_fg = fg_ids.get(path).copied().flatten().is_some();
        if items.len() == 1 + usize::from(has_fg) {
            two += 1;
        }
        if i.chroma < 1.0 {
            grey_want += items.len();
            grey_drawn += items
                .iter()
                .filter(|it| {
                    d.materials
                        .get(it.material)
                        .is_some_and(|m| m.keywords.iter().any(|k| k == "GRAYSCALE"))
                })
                .count();
        }
    }
    println!(
        "  draw: {} items; {two}/{} shown icons draw background + foreground; {grey_drawn}/{grey_want} grey draws with GRAYSCALE",
        d.list.items.len(),
        placed.len()
    );
    check(two == placed.len(), "each shown icon draws its graphics");
    check(grey_drawn == grey_want, "grey icons draw with GRAYSCALE");
    println!("ui --crafting: {ok}/{all} checks passed");
    Ok(if ok == all {
        println!("RUN OK");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
