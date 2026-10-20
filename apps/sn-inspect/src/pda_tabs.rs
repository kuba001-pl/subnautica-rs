//! `ui --pda` (M9u8a4b, `docs/formats/ui.md` § The PDA's toolbar): the
//! PDA screen's tab bar, headless. `uGUI_Toolbar` read from the screen
//! prefab, its icons made by code (`toolbar_icon_nodes`) and grafted under
//! it, then `uGUI_PDA`'s calls (`sn_ui::pda`, `sn_ui::toolbar`) through a
//! script, checked against values computed here apart from the rules:
//! - after `Initialize`: the bar's width n·bg.x + (n − 1)·step, each
//!   icon's place (x = ½·bg.x + i·(bg.x + step), y = ½·bg.y from the
//!   bar's lower left corner) and size (`buttonSize`) in the laid-out
//!   tree, its sprites; the marker hidden;
//! - `OnOpenPDA`: the previous tab (Inventory) selected, the marker at
//!   the icon's `localPosition` + `selectionMarkerOffset`;
//! - clicks on icons 1, 2, 3, 4, 5, 0, 3: each from the icon's centre
//!   projected to a 1920 × 1080 screen with the PDA's pose at rest, back
//!   through the camera's ray onto the canvas (the client's path), hit
//!   tested; each opens its tab: every tab's content alpha against the
//!   expected table, the selected sprite on that icon only, the marker
//!   moved; a right click and a click beside the bar change nothing;
//! - `OnClosePDA` / `OnPDAClosed`, then `OnOpenPDA` opens the last tab
//!   clicked (`tabPrev`);
//! - drawn open: one draw per icon graphic, centred on its icon; what is
//!   drawn over the icons (a stand-in for `GraphicRaycaster`'s blocking,
//!   which we do not port) listed.

use std::collections::HashMap;
use std::process::ExitCode;

use sn_assets::{Assets, GROUP_TAB, GlyphAtlases, UiScene};
use sn_install::GameData;
use sn_ui::draw::{apply, invert, to_ancestor};
use sn_ui::layout::{Rect, Tree, V2};
use sn_ui::pda::{PdaScreen, REGULAR_TABS, Tab, TabShow};
use sn_ui::scaler::{Placement, UiCamera};
use sn_ui::toolbar::{Toolbar, ToolbarIcon, ToolbarSprites};

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const FIELD_OF_VIEW: f32 = 60.0;
/// The canvas at rest in the camera's space (M9u8a2 / M9u8a4a's logs).
const POSITION: [f32; 3] = [0.0217, -0.0012, 0.1874];
/// The clicks, by icon.
const CLICKS: [usize; 7] = [1, 2, 3, 4, 5, 0, 3];

struct Checks {
    ok: usize,
    all: usize,
}

impl Checks {
    fn check(&mut self, good: bool, what: &str) {
        self.all += 1;
        if good {
            self.ok += 1;
        } else {
            println!("  FAIL: {what}");
        }
    }

    fn near(&mut self, got: V2, want: V2, what: &str) {
        let good = (got[0] - want[0]).abs() < 1e-3 && (got[1] - want[1]).abs() < 1e-3;
        self.check(good, &format!("{what}: {got:?}, expected {want:?}"));
    }
}

/// A node's point `p` (its own space) in the canvas root's space.
fn in_canvas(tree: &Tree, node: usize, root: usize, p: V2) -> V2 {
    let q = apply(&to_ancestor(tree, node, root), [p[0], p[1], 0.0]);
    [q[0], q[1]]
}

/// Each tab's content shown (alpha 1 or the object active), by the
/// screen's state.
fn shown_tabs(s: &PdaScreen) -> Vec<Tab> {
    s.config
        .tabs
        .iter()
        .filter(|t| match t.show {
            TabShow::Content(n) => s.state.group_alpha.get(&n) == Some(&1.0),
            TabShow::Object(n) => s.state.active.get(&n) == Some(&true),
        })
        .map(|t| t.tab)
        .collect()
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
    let tree = assets.pda_screen_tree()?;
    let config = assets.pda_screen_config(&layouts, &tree)?;
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let mut c = Checks { ok: 0, all: 0 };

    let tb = assets.pda_toolbar(&layouts, &tree)?;
    let t = &tb.config;
    println!(
        "uGUI_Toolbar: {} (layer {}); backgroundSize {:?}, foregroundSize {:?}, buttonSize {:?}, buttonStep {}, selectionMarker {:?} offset {:?}; notification anchor {} offset {:?}",
        tree.nodes[t.node].path,
        tree.nodes[t.node].layer,
        t.background_size,
        t.foreground_size,
        t.button_size,
        t.button_step,
        t.marker.map(|m| &tree.nodes[m].path),
        t.marker_offset,
        tb.notification_anchor,
        tb.notification_offset
    );
    for (what, (o, name, size)) in [("normal", &tb.normal), ("selected", &tb.selected)] {
        // `SLICE_9_GRID` only for sprites `SpriteManager.GetResource`
        // loaded (from `Resources`, i.e. `resources.assets`).
        println!(
            "  backgroundSprite{what}: {name:?} {size:?} in {} ({})",
            o.file.name,
            if o.file.name.eq_ignore_ascii_case("resources.assets") {
                "a Resources file: SLICE_9_GRID if GetResource loaded it"
            } else {
                "not a Resources file: SLICE_9_GRID off"
            }
        );
    }
    c.check(
        !tb.normal
            .0
            .file
            .name
            .eq_ignore_ascii_case("resources.assets")
            && !tb
                .selected
                .0
                .file
                .name
                .eq_ignore_ascii_case("resources.assets"),
        "the background sprites are not Resources sprites (SLICE_9_GRID off)",
    );

    // The canvas placed as the client places it at rest (its stored size
    // and scale; no rotation: M9u8a2 measured 0.9°).
    let stored = scene
        .stored_rect(config.root)
        .cloned()
        .ok_or("the screen's root has no rect")?;
    let placement = Placement {
        position: POSITION,
        rotation: [0.0, 0.0, 0.0, 1.0],
        size: stored.size_delta,
        scale: stored.scale,
        scale_factor: 1.0,
    };
    let placements = HashMap::from([(config.root, placement)]);
    let camera = UiCamera::game(FIELD_OF_VIEW, 0.03, 1700.0);

    let mut screen = PdaScreen::new(config.clone());
    let before = scene.layout_for(&assets, &set, &placements, &screen.state)?;
    let mut tc = tb.config.clone();
    tc.lay_out(&before);
    println!(
        "  laid out: height {}, stored sizeDelta {:?}, parent span x {}, marker anchor {:?}",
        tc.height, tc.size_delta, tc.parent_span_x, tc.marker_anchor
    );

    // The tabs' sprites (`SpriteManager.Get(Group.Tab, "Tab<name>")`).
    let mut tab_sprites = Vec::new();
    for tab in REGULAR_TABS {
        let name = format!("Tab{tab:?}");
        match sprites.get_no_default(GROUP_TAB, &name) {
            Some(s) => {
                let size = [s.sprite.rect[2], s.sprite.rect[3]];
                println!("  {name}: {size:?}");
                tab_sprites.push((tab, (scene.sprite_id(&s.object), size)));
            }
            None => println!("  {name}: not in the Tabs atlas"),
        }
    }
    c.check(
        tab_sprites.len() == REGULAR_TABS.len(),
        "every regular tab has its sprite",
    );
    let tab_size: HashMap<Tab, V2> = tab_sprites.iter().map(|(t, (_, s))| (*t, *s)).collect();

    let material = assets.icon_material(&catalog, &layouts)?;
    let template = UiScene::from_code(
        "ToolbarIcon",
        sn_assets::toolbar_icon_nodes(tree.nodes[tc.node].layer, &material),
    )?;
    let normal = scene.sprite_id(&tb.normal.0);
    let selected = scene.sprite_id(&tb.selected.0);
    let toolbar = Toolbar::new(
        tc.clone(),
        ToolbarSprites {
            normal: (normal, tb.normal.2),
            selected: (selected, tb.selected.2),
        },
    );
    let stored_nodes = scene.node_count();
    let mut failed = None;
    let mut make = || match scene.graft(&template, tc.node) {
        Ok(n) => ToolbarIcon {
            root: n,
            background: n + 1,
            foreground: n + 2,
        },
        Err(e) => {
            failed = Some(e);
            ToolbarIcon {
                root: usize::MAX,
                background: usize::MAX,
                foreground: usize::MAX,
            }
        }
    };
    screen.attach_toolbar(toolbar, tab_sprites, &mut make);
    if let Some(e) = failed {
        return Err(format!("graft ToolbarIcon: {e}"));
    }
    let icons = screen.toolbar.as_ref().ok_or("no toolbar")?.icons.clone();
    for &n in &screen
        .toolbar
        .as_ref()
        .ok_or("no toolbar")?
        .events
        .last_sibling
    {
        scene.set_as_last_sibling(n);
    }
    println!(
        "  {} icons grafted ({} nodes; {} before)",
        icons.len(),
        scene.node_count() - stored_nodes,
        stored_nodes
    );
    c.check(icons.len() == 6, "6 icons");

    // After `Initialize`: the rule, computed here.
    let n = REGULAR_TABS.len() as f32;
    let (bg, step) = (tc.background_size, tc.button_step);
    let width = n * bg[0] + (n - 1.0) * step;
    let l = scene.layout_for(&assets, &set, &placements, &screen.state)?;
    let bar = l.rect(tc.node);
    c.check(
        (bar.width - width).abs() < 1e-3,
        &format!("bar width {}, expected {width}", bar.width),
    );
    println!("  bar rect {bar:?}");
    let fit = |want: V2, have: V2| {
        let k = (want[0] / have[0]).min(want[1] / have[1]);
        [have[0] * k, have[1] * k]
    };
    let mut centres = Vec::new();
    for (i, icon) in icons.iter().enumerate() {
        let want = [
            bar.x + 0.5 * bg[0] + i as f32 * (bg[0] + step),
            bar.y + 0.5 * bg[1],
        ];
        c.near(
            l.local_position(icon.root),
            want,
            &format!("icon {i} place"),
        );
        c.near(
            l.rect(icon.root).size(),
            tc.button_size,
            &format!("icon {i} size"),
        );
        c.near(
            l.rect(icon.background).size(),
            fit(bg, tb.normal.2),
            &format!("icon {i} background size (aspect kept)"),
        );
        let tab = REGULAR_TABS[i];
        if let Some(s) = tab_size.get(&tab) {
            c.near(
                l.rect(icon.foreground).size(),
                fit(tc.foreground_size, *s),
                &format!("icon {i} foreground size"),
            );
        }
        c.check(
            screen.state.sprite.get(&icon.background) == Some(&Some(normal)),
            &format!("icon {i} normal background"),
        );
        centres.push(in_canvas(&l, icon.root, config.root, [0.0, 0.0]));
    }
    if let Some(m) = tc.marker {
        c.check(
            screen.state.active.get(&m) == Some(&false),
            "the marker hidden after Initialize",
        );
    }

    // `OnOpenPDA(None)`: the previous tab, Inventory.
    screen.on_open_pda(None);
    let check_open = |c: &mut Checks,
                      s: &PdaScreen,
                      scene: &mut UiScene,
                      expect_tab: Tab,
                      what: &str|
     -> Result<()> {
        let i = REGULAR_TABS
            .iter()
            .position(|&t| t == expect_tab)
            .ok_or("not a regular tab")?;
        c.check(
            s.tab_open == Some(expect_tab),
            &format!("{what}: tab {:?}", s.tab_open),
        );
        c.check(
            shown_tabs(s) == [expect_tab],
            &format!(
                "{what}: shown {:?}, expected [{expect_tab:?}]",
                shown_tabs(s)
            ),
        );
        let alpha = |n: usize| s.state.group_alpha.get(&n).copied();
        c.check(
            alpha(s.config.canvas_group) == Some(1.0)
                && alpha(s.config.content) == Some(1.0)
                && alpha(s.config.toolbar_group) == Some(1.0),
            &format!("{what}: canvas, content and toolbar shown"),
        );
        for (k, icon) in icons.iter().enumerate() {
            let want = if k == i { selected } else { normal };
            c.check(
                s.state.sprite.get(&icon.background) == Some(&Some(want)),
                &format!("{what}: icon {k}'s background"),
            );
        }
        if let Some(m) = tc.marker {
            let l = scene.layout_for(&assets, &set, &placements, &s.state)?;
            let p = l.local_position(icons[i].root);
            c.near(
                l.local_position(m),
                [p[0] + tc.marker_offset[0], p[1] + tc.marker_offset[1]],
                &format!("{what}: the marker at icon {i} + offset"),
            );
            c.check(
                s.state.active.get(&m) == Some(&true),
                &format!("{what}: marker shown"),
            );
        }
        Ok(())
    };
    check_open(&mut c, &screen, &mut scene, Tab::Inventory, "OnOpenPDA")?;

    // Clicks, through the screen and back.
    let bar_from_canvas = invert(&to_ancestor(&l, tc.node, config.root)).ok_or("singular")?;
    let world_from_canvas = placement.world_from_canvas();
    let click = |screen: &mut PdaScreen, at: V2| -> Result<(Option<usize>, V2)> {
        let w = apply(&world_from_canvas, [at[0], at[1], 0.0]);
        let pixel = camera.project(w, SCREEN).ok_or("behind the camera")?;
        let back = placement
            .ray_point(camera.position, camera.screen_ray(pixel, SCREEN))
            .ok_or("the ray misses the canvas")?;
        let p = apply(&bar_from_canvas, [back[0], back[1], 0.0]);
        let tb = screen.toolbar.as_ref().ok_or("no toolbar")?;
        let r = Rect {
            width: tb.width,
            ..bar
        };
        Ok((tb.hit([p[0], p[1]], r), pixel))
    };
    for &i in &CLICKS {
        let (hit, pixel) = click(&mut screen, centres[i])?;
        c.check(hit == Some(i), &format!("click on icon {i}: hit {hit:?}"));
        let opened = hit.is_some_and(|h| screen.on_toolbar_click(h, 0));
        println!(
            "  click at pixel ({:.1}, {:.1}): icon {hit:?}, {:?} opened {opened}",
            pixel[0], pixel[1], screen.tab_open
        );
        check_open(
            &mut c,
            &screen,
            &mut scene,
            REGULAR_TABS[i],
            &format!("click {i}"),
        )?;
    }
    // A right click on another tab, and a click left of the bar.
    let (hit, _) = click(&mut screen, centres[0])?;
    c.check(
        hit == Some(0) && !screen.on_toolbar_click(0, 1),
        "a right click opens nothing",
    );
    let beside = in_canvas(
        &l,
        tc.node,
        config.root,
        [bar.x - 20.0, bar.y + 0.5 * bg[1]],
    );
    let (hit, _) = click(&mut screen, beside)?;
    c.check(hit.is_none(), &format!("a click beside the bar: {hit:?}"));
    let last_tab = REGULAR_TABS[CLICKS[CLICKS.len() - 1]];
    check_open(
        &mut c,
        &screen,
        &mut scene,
        last_tab,
        "after the right and outside clicks",
    )?;

    // Drawn open: one draw per icon graphic, centred on its icon.
    let d = scene.draw(
        &assets,
        &fonts,
        &set,
        &mut atlases,
        &placements,
        &screen.state,
    )?;
    let items = &d.list.items;
    let mut icon_draws = 0;
    let mut last = 0;
    for (k, it) in items.iter().enumerate() {
        let Some(i) = icons
            .iter()
            .position(|ic| ic.background == it.node || ic.foreground == it.node)
        else {
            continue;
        };
        icon_draws += 1;
        last = k;
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for v in &it.vertices {
            for a in 0..2 {
                lo[a] = lo[a].min(v.pos[a]);
                hi[a] = hi[a].max(v.pos[a]);
            }
        }
        let mid = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
        // A sprite mesh is trimmed to its pixels: centred within 4 units.
        let good = (mid[0] - centres[i][0]).abs() < 4.0 && (mid[1] - centres[i][1]).abs() < 4.0;
        c.check(
            good,
            &format!(
                "icon {i}'s {} draw centred: {mid:?}, icon at {:?}",
                it.kind, centres[i]
            ),
        );
    }
    let graphics = icons.len() * 2;
    c.check(
        icon_draws == graphics,
        &format!("{icon_draws} icon draws, expected {graphics}"),
    );
    let over: Vec<String> = items
        .iter()
        .skip(last + 1)
        .filter(|it| it.alpha > 0.0)
        .filter(|it| {
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for v in &it.vertices {
                for a in 0..2 {
                    lo[a] = lo[a].min(v.pos[a]);
                    hi[a] = hi[a].max(v.pos[a]);
                }
            }
            centres
                .iter()
                .any(|p| p[0] >= lo[0] && p[0] <= hi[0] && p[1] >= lo[1] && p[1] <= hi[1])
        })
        .map(|it| scene.tree.nodes[it.node].path.clone())
        .collect();
    println!(
        "  drawn open: {} items, {icon_draws} of the icons; drawn after them over an icon's centre: {over:?}",
        items.len()
    );

    // Close, then open again: the last tab clicked.
    screen.on_close_pda();
    let tbr = screen.toolbar.as_ref().ok_or("no toolbar")?;
    c.check(tbr.selected.is_none(), "OnClosePDA: nothing selected");
    c.check(tbr.icons == icons, "OnClosePDA: the same icon nodes");
    c.check(shown_tabs(&screen).is_empty(), "OnClosePDA: no tab shown");
    c.check(
        screen.state.group_alpha.get(&config.content) == Some(&0.0),
        "OnClosePDA: content hidden",
    );
    if let Some(m) = tc.marker {
        c.check(
            screen.state.active.get(&m) == Some(&false),
            "OnClosePDA: marker hidden",
        );
    }
    screen.on_pda_closed();
    c.check(
        screen.state.group_alpha.get(&config.canvas_group) == Some(&0.0),
        "OnPDAClosed: canvas hidden",
    );
    screen.on_open_pda(None);
    check_open(
        &mut c,
        &screen,
        &mut scene,
        last_tab,
        "OnOpenPDA again (tabPrev)",
    )?;

    println!("{} of {} checks ok", c.ok, c.all);
    if c.ok == c.all {
        println!("RUN OK");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("RUN FAILED");
        Ok(ExitCode::FAILURE)
    }
}
