//! `ui --notifications` (M9u8b5a, `docs/formats/ui.md` § Notifications):
//! `uGUI.prefabNotificationLabel` and `sn_ui::notifications` on the PDA
//! screen, headless, checked against values written before the run:
//! - the label prefab: its `uGUI_NotificationLabel`'s `background` and
//!   `text` nodes, the text's stored colour;
//! - grafted under the toolbar's inventory icon (`SetNotificationsAmount`:
//!   "1", anchor `notificationAnchor`, offset `notificationOffset`) and
//!   under an item icon (`SetNotificationProgress(0.5)`: "+", upper
//!   right, offset (−10, −10), alpha sin(π/4)): each label's pivot at its
//!   parent's anchor point + the offset, both graphics drawn, the second's
//!   vertex alpha round(255 · sin(π/4)) = 180;
//! - a replay at 60 frames a second: a pick-up's `Add(Inventory, id, 4)`
//!   with the icon as its target, the PDA closed 3 s (count 1, progress 1
//!   each frame), then open: count 1 until 4 s of visible time (± a
//!   frame), then 0. Visibility from the scene (the icon active, its
//!   inherited alpha above 0).
//! - M9u8b5b: `Resources` `Sprites/NotificationOverlay` is a texture.

use std::collections::HashMap;
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, UiScene, UiState};
use sn_install::GameData;
use sn_ui::draw::{apply, to_ancestor};
use sn_ui::item_icons::IconNodes;
use sn_ui::layout::{Tree, V2};
use sn_ui::notifications::{
    Group, ICON_LABEL_ANCHOR, ICON_LABEL_OFFSET, LabelNodes, Notifications, PICKUP_DURATION,
    SYMBOL, anchor_point, label_alpha, label_state,
};
use sn_ui::pda::{PdaScreen, REGULAR_TABS, Tab};
use sn_ui::scaler::Placement;
use sn_ui::toolbar::{Toolbar, ToolbarIcon, ToolbarSprites};

use crate::Result;

/// The canvas at rest in the camera's space (as `ui --pda`).
const POSITION: [f32; 3] = [0.0217, -0.0012, 0.1874];
const FRAME: f32 = 1.0 / 60.0;

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
}

fn in_canvas(tree: &Tree, node: usize, root: usize, p: V2) -> V2 {
    let q = apply(&to_ancestor(tree, node, root), [p[0], p[1], 0.0]);
    [q[0], q[1]]
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let tags = sn_assets::physics_settings(&assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let catalog = assets.catalog()?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let mut c = Checks { ok: 0, all: 0 };

    // The prefab.
    let ugui = assets.ui_tree("uGUI.prefab", &assets.ugui_root(&catalog)?)?;
    let (nodes, template) =
        assets.notification_label(&layouts, &ugui, &fonts, &tags.sorting_layers)?;
    println!(
        "NotificationLabel: {} nodes; background {} ({:?}), text {} ({:?}); colours {:?} / {:?}",
        template.node_count(),
        nodes.background,
        template.tree.nodes[nodes.background].name,
        nodes.text,
        template.tree.nodes[nodes.text].name,
        nodes.background_color,
        nodes.text_color
    );
    c.check(
        template.node_count() == 2 && nodes.background != nodes.text,
        "two nodes, background and text apart",
    );
    c.check(
        template.text_params(nodes.text).is_some(),
        "the text node is a TextMeshProUGUI",
    );

    // `iconMaterial`'s overlay (M9u8b5b).
    let overlay = assets.resource_texture("Sprites/NotificationOverlay")?;
    println!(
        "  Sprites/NotificationOverlay: {:?}",
        overlay
            .as_ref()
            .map(|o| (&o.file.name, o.data().map(|(i, d)| (i.class_id, d.len()))))
    );
    c.check(
        overlay.is_some(),
        "Resources Sprites/NotificationOverlay is a Texture2D",
    );

    // The PDA screen with its toolbar.
    let tree = assets.pda_screen_tree()?;
    let config = assets.pda_screen_config(&layouts, &tree)?;
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let stored = scene
        .stored_rect(config.root)
        .cloned()
        .ok_or("the screen's root has no rect")?;
    let placements = HashMap::from([(
        config.root,
        Placement {
            position: POSITION,
            rotation: [0.0, 0.0, 0.0, 1.0],
            size: stored.size_delta,
            scale: stored.scale,
            scale_factor: 1.0,
        },
    )]);
    let mut screen = PdaScreen::new(config.clone());
    let tb = assets.pda_toolbar(&layouts, &tree)?;
    let mut tc = tb.config.clone();
    tc.lay_out(&scene.layout_for(&assets, &set, &placements, &screen.state)?);
    let material = assets.icon_material(&catalog, &layouts)?;
    let layer = tree.nodes[tc.node].layer;
    let toolbar_template = UiScene::from_code(
        "ToolbarIcon",
        sn_assets::toolbar_icon_nodes(layer, &material),
    )?;
    let toolbar = Toolbar::new(
        tc.clone(),
        ToolbarSprites {
            normal: (scene.sprite_id(&tb.normal.0), tb.normal.2),
            selected: (scene.sprite_id(&tb.selected.0), tb.selected.2),
        },
    );
    let mut failed = None;
    let mut make = || match scene.graft(&toolbar_template, tc.node) {
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
    screen.attach_toolbar(toolbar, Vec::new(), &mut make);
    if let Some(e) = failed {
        return Err(format!("graft ToolbarIcon: {e}"));
    }
    screen.on_open_pda(None);
    let inventory = REGULAR_TABS
        .iter()
        .position(|t| *t == Tab::Inventory)
        .ok_or("no inventory tab")?;
    let tab_icon = screen.toolbar.as_ref().ok_or("no toolbar")?.icons[inventory];

    // An item icon under the screen's root, 66 × 66.
    let item_template = UiScene::from_code(
        "Item Icon",
        sn_assets::item_icon_nodes(layer, &material, false),
    )?;
    let n = scene.graft(&item_template, config.root)?;
    let icon = IconNodes {
        root: n,
        background: n + 1,
        foreground: n + 2,
    };

    // The labels.
    let graft_label = |scene: &mut UiScene, parent: usize, anchor: V2| -> Result<LabelNodes> {
        let root = scene.graft(&template, parent)?;
        scene.set_anchors(root, anchor);
        Ok(LabelNodes {
            root,
            background: root + nodes.background,
            text: root + nodes.text,
        })
    };
    let toolbar_anchor = anchor_point(tb.notification_anchor);
    let count_label = graft_label(&mut scene, tab_icon.root, toolbar_anchor)?;
    let plus_label = graft_label(&mut scene, icon.root, anchor_point(ICON_LABEL_ANCHOR))?;
    let mut st = screen.state.clone();
    st.transform.entry(icon.root).or_default().size_delta = Some([66.0, 66.0]);
    label_state(
        &mut st,
        count_label,
        "1",
        1.0,
        tb.notification_offset,
        nodes.text_color,
        true,
    );
    let plus_alpha = label_alpha(0.5);
    label_state(
        &mut st,
        plus_label,
        SYMBOL,
        plus_alpha,
        ICON_LABEL_OFFSET,
        nodes.text_color,
        false,
    );
    let l = scene.layout_for(&assets, &set, &placements, &st)?;
    for (what, label, parent, anchor, offset) in [
        (
            "count",
            count_label,
            tab_icon.root,
            toolbar_anchor,
            tb.notification_offset,
        ),
        (
            "plus",
            plus_label,
            icon.root,
            anchor_point(ICON_LABEL_ANCHOR),
            ICON_LABEL_OFFSET,
        ),
    ] {
        let r = l.rect(parent);
        let want = in_canvas(
            &l,
            parent,
            config.root,
            [
                r.x + anchor[0] * r.width + offset[0],
                r.y + anchor[1] * r.height + offset[1],
            ],
        );
        let got = in_canvas(&l, label.root, config.root, [0.0, 0.0]);
        println!(
            "  {what} label: pivot {got:?} (rule {want:?}), rect {:?}, parent {:?}",
            l.rect(label.root),
            r.size()
        );
        c.check(
            (got[0] - want[0]).abs() < 1e-3 && (got[1] - want[1]).abs() < 1e-3,
            &format!("{what} label at its parent's anchor + offset"),
        );
        c.check(
            l.active(label.root) && l.rect(label.root).width > 0.0,
            &format!("{what} label shown with a size"),
        );
    }
    let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &st)?;
    for (what, label, alpha) in [
        ("count", count_label, 1.0),
        ("plus", plus_label, plus_alpha),
    ] {
        let want = (alpha * 255.0f32).round() as u8;
        for (part, node) in [("background", label.background), ("text", label.text)] {
            let items: Vec<_> = d.list.items.iter().filter(|i| i.node == node).collect();
            let alphas: Vec<u8> = items
                .iter()
                .flat_map(|i| i.vertices.iter().map(|v| v.color[3]))
                .collect();
            let lo = alphas.iter().copied().min();
            let hi = alphas.iter().copied().max();
            println!(
                "  {what} {part}: {} draws, vertex alpha {lo:?}..{hi:?} (want {want})",
                items.len()
            );
            // The text's own colour has alpha 1 (stored); the background's
            // white.
            c.check(
                !items.is_empty() && lo.is_some_and(|a| a.abs_diff(want) <= 1) && lo == hi,
                &format!("{what} {part} drawn at alpha {want}"),
            );
        }
    }

    // The replay.
    // `IsVisible`: the icon and its foreground active (with the state's
    // changes), the foreground's inherited alpha above 0.
    let visible = |scene: &UiScene, s: &UiState, _t: u32| {
        let active = |mut node: usize| {
            for _ in 0..scene.tree.nodes.len() {
                let Some(n) = scene.tree.nodes.get(node) else {
                    return false;
                };
                if !s.active.get(&node).copied().unwrap_or(n.active_self) {
                    return false;
                }
                match n.parent {
                    Some(p) => node = p,
                    None => return true,
                }
            }
            false
        };
        active(icon.root)
            && active(icon.foreground)
            && scene.inherited_alpha(icon.foreground, s) > 0.0
    };
    screen.on_close_pda();
    screen.on_pda_closed();
    let closed_visible = visible(&scene, &screen.state, 1);
    let mut m = Notifications::<u32>::new();
    let key = (Group::Inventory, 6);
    let added = m.add(key, PICKUP_DURATION);
    m.register(key, 1, &|t| visible(&scene, &screen.state, t));
    let mut closed_ok = true;
    for _ in 0..180 {
        let p = m.late_update(FRAME, &|_| closed_visible);
        closed_ok &= p == [(1, 1.0)] && m.count(Group::Inventory) == 1;
    }
    println!(
        "  replay: added {added:?}; closed 3 s: visible {closed_visible}, count {}",
        m.count(Group::Inventory)
    );
    c.check(
        !closed_visible,
        "the icon is not visible with the PDA closed",
    );
    c.check(closed_ok, "closed: count 1 and progress 1 every frame");
    screen.on_open_pda(None);
    let open_visible = visible(&scene, &screen.state, 1);
    c.check(open_visible, "the icon is visible with the PDA open");
    let mut frames = 0;
    let mut last = Vec::new();
    while m.count(Group::Inventory) > 0 && frames < 600 {
        last = m.late_update(FRAME, &|_| open_visible);
        frames += 1;
    }
    let seen = frames as f32 * FRAME;
    println!("  open: count 0 after {frames} frames ({seen:.4} s), last progress {last:?}");
    c.check(
        (seen - PICKUP_DURATION).abs() <= FRAME + 1e-4 && last == [(1, 0.0)],
        "cleared after 4 s of visible time (± a frame), the label to 0",
    );

    println!("{} of {} checks ok", c.ok, c.all);
    if c.ok == c.all {
        println!("RUN OK");
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}
