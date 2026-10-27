//! `ui --tooltip` (M9u8b3b, `docs/formats/ui.md` § The tooltip): the
//! tooltip, headless.
//!
//! - The prefab: `uGUI_Tooltip`'s nodes and every node under `uGUI/Tooltip`
//!   (rects, text parameters) dumped.
//! - The texts (M9u8b3b1): a scripted inventory (the player's grid:
//!   Flare, FilteredWater, NutrientBlock, FirstAidKit, Titanium, Battery;
//!   the Tank in its slot; no storage open, outside the lifepod, survival).
//!   Each item hovered in turn goes through `sn_ui::tooltip::inventory_item`
//!   with its actions from `sn_sim::inventory` and its values from its
//!   prefab. Each text line is checked against one built here from the
//!   language file (the tags, the title, the value lines with the formats
//!   written out here, the description) and the actions against the table
//!   DESIGN's M9u8b3b1 row wrote before the run.
//! - Layout and placement (M9u8b3b2): each item's texts through
//!   `TooltipLayout` (TMP's preferred sizes of the prefab's texts), laid
//!   out in the `uGUI` tree: the root's size, the texts' rects at their
//!   places and inside the root. Then `update_position` on the PDA
//!   screen's canvas (its stored rect and scale, [`PDA_DEPTH`] in front of
//!   the camera as the client's PDA is) with the pointer at the canvas's
//!   centre, near its right edge, near its bottom and in its bottom right
//!   corner: the rect on the side the rule says (worked here), inside the
//!   canvas and clear of the pointer.

use std::collections::HashMap;
use std::process::ExitCode;

use sn_assets::{Assets, UiScene, UiState};
use sn_install::GameData;
use sn_sim::inventory::{self as inv, Button, DropPlace, Inventory, Item, ItemId, KnownTech};
use sn_ui::layout::{Rect, Tree};
use sn_ui::scaler::{DEFAULT_UI_SCALE, Placement};
use sn_ui::text::FontSet;
use sn_ui::tooltip::{
    MARGIN, MAX_TEXT_WIDTH, PointerOnCanvas, TooltipConfig, TooltipLayout, TooltipStrings,
    inventory_item, update_position,
};

use crate::Result;

/// The PDA screen's distance in front of the camera in the client's runs
/// (its logged pose, M9u8a).
const PDA_DEPTH: f32 = 0.1874;

/// The scripted grid, in order (ids 1…).
const GRID: [&str; 6] = [
    "Flare",
    "FilteredWater",
    "NutrientBlock",
    "FirstAidKit",
    "Titanium",
    "Battery",
];
/// The equipped item and its slot (id 100).
const EQUIPPED: (&str, &str) = ("Tank", "Tank");

/// The rule written before the run: per item, the bind hint and the
/// actions (button, language key) in the order written.
fn expected_actions(name: &str) -> (bool, &'static [(usize, &'static str)]) {
    match name {
        "Flare" => (true, &[(1, "Drop")]),
        "FilteredWater" | "NutrientBlock" => (false, &[(0, "Eat"), (1, "Drop")]),
        "FirstAidKit" => (false, &[(0, "Use"), (1, "Drop")]),
        "Titanium" | "Battery" => (false, &[(1, "Drop")]),
        "Tank" => (false, &[(0, "Unequip")]),
        _ => (false, &[]),
    }
}

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

/// C#'s `+#;-#;+0` for an integer, written out.
fn signed(v: i64) -> String {
    match v {
        0 => "+0".into(),
        v if v > 0 => format!("+{v}"),
        v => v.to_string(),
    }
}

fn title_line(t: &str) -> String {
    format!("<size=25><color=#ffffffff>{t}</color></size>")
}

fn desc_line(t: &str) -> String {
    format!("<size=20><color=#DDDEDEFF>{t}</color></size>")
}

fn action_line(key: &str, word: &str) -> String {
    format!("<size=20><color=#ffffffff>{key}</color> - <color=#00ffffff>{word}</color></size>")
}

/// Dumps the prefab's nodes under `uGUI/Tooltip` (laid out as stored).
fn dump_prefab(tree: &sn_assets::UiTree, scene: &UiScene, config: &TooltipConfig, laid: &Tree) {
    println!("uGUI_Tooltip: {config:?}");
    for (i, n) in tree.nodes.iter().enumerate() {
        let mut p = Some(i);
        let mut under = false;
        for _ in 0..tree.nodes.len() {
            let Some(k) = p else { break };
            if k == config.root {
                under = true;
                break;
            }
            p = tree.nodes.get(k).and_then(|n| n.parent);
        }
        if !under {
            continue;
        }
        let comps: Vec<String> = n
            .components
            .iter()
            .map(|c| c.script.clone().unwrap_or_else(|| c.class_id.to_string()))
            .collect();
        println!("  [{i}] {} active {} {comps:?}", n.path, n.active_self);
        if let Some(r) = scene.stored_rect(i) {
            println!(
                "      anchors {:?}–{:?} pos {:?} size {:?} pivot {:?} scale {:?}; laid {:?}",
                r.anchor_min,
                r.anchor_max,
                r.anchored_position,
                r.size_delta,
                r.pivot,
                r.scale,
                laid.rect(i)
            );
        }
        if let Some(t) = scene.text_params(i) {
            println!(
                "      text font {} size {} auto {} align {}/{} wrap {} overflow {} margin {:?} rich {} spacing {}/{}/{} text {:?}",
                t.font,
                t.font_size,
                t.auto_size,
                t.h_align,
                t.v_align,
                t.word_wrap,
                t.overflow,
                t.margin,
                t.rich_text,
                t.line_spacing,
                t.paragraph_spacing,
                t.character_spacing,
                t.text
            );
        }
    }
}

/// The PDA screen's canvas: its stored rect (about its pivot) and scale.
struct Canvas {
    rect: Rect,
    scale: [f32; 2],
}

/// M9u8b3b2's checks for one item's texts.
#[allow(clippy::too_many_arguments)]
fn check_layout(
    c: &mut Checks,
    name: &str,
    assets: &Assets,
    set: &FontSet,
    scene: &mut UiScene,
    config: &TooltipConfig,
    canvas: &Canvas,
    (prefix, postfix): (&str, &str),
) -> Result<()> {
    // `OnLayout`: TMP's preferred sizes of the prefab's two texts.
    let pref = |scene: &UiScene, node: usize, text: &str, width: f32| -> Result<[f32; 2]> {
        scene
            .preferred_size(set, node, text, width)
            .ok_or_else(|| format!("node {node}: no text"))
    };
    let widths = [
        pref(scene, config.text_prefix, prefix, 0.0)?[0],
        pref(scene, config.text_postfix, postfix, 0.0)?[0],
    ];
    let a = widths[0].max(widths[1]).min(MAX_TEXT_WIDTH);
    let heights = [
        pref(scene, config.text_prefix, prefix, a)?[1],
        pref(scene, config.text_postfix, postfix, a)?[1],
    ];
    let layout = TooltipLayout::new(widths, |_| heights);
    println!(
        "  layout: widths {widths:?} → a {}, heights {heights:?}, size {:?}",
        layout.width, layout.size
    );
    let want_h = 20.0
        + heights[0]
        + if heights[1] > 0.0 { 10.0 } else { 0.0 }
        + heights[1]
        + 20.0;
    c.check(
        layout.width == a && layout.size == [a + 40.0, want_h] && heights[0] > 0.0 && heights[1] > 0.0,
        &format!(
            "{name}: OnLayout's size {:?}, expected [{}, {want_h}]",
            layout.size,
            a + 40.0
        ),
    );
    // Laid out in the tree: the root sized as a canvas, the texts at
    // their places, inside it.
    let mut placements = HashMap::new();
    placements.insert(
        config.root,
        Placement {
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            size: layout.size,
            scale: [1.0; 3],
            scale_factor: 1.0,
        },
    );
    let st = layout.state(config, prefix, postfix);
    let l = scene.layout_for(assets, set, &placements, &st)?;
    let root = l.rect(config.root);
    let top_left = [root.x, root.y + root.height];
    let mut inside = root.width == layout.size[0] && root.height == layout.size[1];
    for (node, at, h) in [
        (config.text_prefix, layout.prefix_at, heights[0]),
        (config.text_postfix, layout.postfix_at, heights[1]),
    ] {
        let p = l.local_position(node);
        let r = l.rect(node);
        inside &= (p[0] - (top_left[0] + at[0])).abs() < 1e-3
            && (p[1] - (top_left[1] + at[1])).abs() < 1e-3
            && (r.width - a).abs() < 1e-3
            && (r.height - h).abs() < 1e-3
            && p[0] >= root.x
            && p[0] + r.width <= root.x + root.width + 1e-3
            && p[1] <= top_left[1] + 1e-3
            && p[1] - r.height >= root.y - 1e-3;
    }
    c.check(
        inside,
        &format!("{name}: the texts laid out at their places inside the root {root:?}"),
    );

    // `UpdatePosition` on the PDA's canvas.
    let r = canvas.rect;
    let k = PDA_DEPTH * config.scale_factor * DEFAULT_UI_SCALE / canvas.scale[0];
    let (w, h) = (layout.size[0] * k, layout.size[1] * k);
    let (mx, my) = (MARGIN[0] * k, MARGIN[1] * k);
    let (right, bottom, top) = (r.x + r.width, r.y, r.y + r.height);
    let centre = [r.x + 0.5 * r.width, r.y + 0.5 * r.height];
    // (where, pointer, the top left by the rule).
    let cases = [
        ("centre", centre, [centre[0] + mx, centre[1] - my]),
        (
            "right edge",
            [right - 50.0, centre[1]],
            [right - 50.0 - mx - w, centre[1] - my],
        ),
        (
            "bottom",
            [centre[0], bottom + 30.0],
            [centre[0] + mx, bottom + 30.0 + my + h],
        ),
        (
            "bottom right corner",
            [right - 50.0, bottom + 30.0],
            [right - 50.0 - mx - w, bottom + 30.0 + my + h],
        ),
    ];
    for (place, at, want) in cases {
        let pose = update_position(
            layout.size,
            &PointerOnCanvas {
                rect: r,
                canvas_scale: canvas.scale,
                pointer: [at[0], at[1], 0.0],
                depth: PDA_DEPTH,
            },
            config.scale_factor,
            DEFAULT_UI_SCALE,
        );
        let p = pose.point;
        let covers = at[0] >= p[0] && at[0] <= p[0] + w && at[1] <= p[1] && at[1] >= p[1] - h;
        let good = (p[0] - want[0]).abs() < 1e-3
            && (p[1] - want[1]).abs() < 1e-3
            && p[0] >= r.x
            && p[0] + w <= right + 1e-3
            && p[1] - h >= bottom - 1e-3
            && p[1] <= top + 1e-3
            && !covers;
        println!(
            "  pointer at the {place} ({:.1}, {:.1}): scale {:.7}, top left ({:.1}, {:.1}), {w:.1} × {h:.1} on the canvas",
            at[0], at[1], pose.scale, p[0], p[1]
        );
        c.check(
            good,
            &format!(
                "{name}: pointer at the {place}: top left {p:?}, expected {want:?}, inside the canvas, clear of the pointer"
            ),
        );
    }
    Ok(())
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let tags = sn_assets::physics_settings(&assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let catalog = assets.catalog()?;
    let ugui = assets.ugui_root(&catalog)?;
    let tree = assets.ui_tree("uGUI.prefab", &ugui)?;
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let config = assets.tooltip_config(&layouts, &tree)?;
    let laid = scene.layout_for(&assets, &set, &HashMap::new(), &UiState::default())?;
    dump_prefab(&tree, &scene, &config, &laid);
    let mut c = Checks { ok: 0, all: 0 };

    // The PDA screen's canvas.
    let pda_tree = assets.pda_screen_tree()?;
    let pda_config = assets.pda_screen_config(&layouts, &pda_tree)?;
    let pda_scene = assets.ui_scene(&layouts, &pda_tree, &fonts, &tags.sorting_layers)?;
    let pr = pda_scene
        .stored_rect(pda_config.root)
        .cloned()
        .ok_or("uGUI_PDAScreen: no RectTransform")?;
    let canvas = Canvas {
        rect: Rect {
            x: -pr.pivot[0] * pr.size_delta[0],
            y: -pr.pivot[1] * pr.size_delta[1],
            width: pr.size_delta[0],
            height: pr.size_delta[1],
        },
        scale: [pr.scale[0], pr.scale[1]],
    };
    println!(
        "PDA canvas: rect {:?}, scale {:?}, {PDA_DEPTH} m in front of the camera",
        canvas.rect, canvas.scale
    );

    // The strings: the language file and the buttons.
    let language = sn_assets::language(game, "English")?;
    let lang = |k: &str| language.get(k).cloned().unwrap_or_else(|| k.to_string());
    let mut buttons: [String; 4] = Default::default();
    for (b, name) in ["UISubmit", "UICancel", "UIClear", "UIAssign"].iter().enumerate() {
        let t = sn_assets::format_button(game, &set, &language, name)?;
        println!("FormatButton({name}): {:?}", t.text);
        buttons[b] = t.text;
    }
    c.check(
        buttons[0].contains("name=\"MouseButtonLeft\""),
        "button 0 is the left mouse button's sprite",
    );
    c.check(
        buttons[1].contains("name=\"MouseButtonRight\""),
        "button 1 is the right mouse button's sprite",
    );
    let strings = TooltipStrings::new(&|k| lang(k), buttons.clone());

    // The scripted inventory.
    let dll = sn_assets::read_assembly(game)?;
    let code = sn_assets::game_code(&dll)?;
    let tech = |name: &str| -> Result<i32> {
        code.tech_types
            .iter()
            .find(|(_, n)| n == name)
            .map(|(v, _)| *v)
            .ok_or_else(|| format!("no tech type {name}"))
    };
    let hand = {
        let asm = sn_dotnet::Assembly::parse(&dll).map_err(|e| e.to_string())?;
        sn_dotnet::enum_values(&asm, "", "EquipmentType")
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|(n, _)| n == "Hand")
            .and_then(|(_, v)| i32::try_from(v).ok())
            .ok_or("EquipmentType.Hand not found")?
    };
    let (rules, _) = sn_assets::item_rules(game, &assets, &dll)?;
    let class_paths: HashMap<String, String> = game
        .read_prefab_database_ordered()
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    let mut inventory = Inventory::new(
        rules.inventory_size,
        &rules.slots,
        &rules.player_slots,
        KnownTech::default(),
    );
    let mut hovered: Vec<(u64, &str, bool)> = Vec::new();
    for (k, name) in GRID.iter().enumerate() {
        let id = k as u64 + 1;
        inventory
            .container
            .add(&rules.techs, Item::new(id, tech(name)?))
            .map_err(|_| format!("{name}: no room in the grid"))?;
        hovered.push((id, name, true));
    }
    inventory
        .equipment
        .add_to(&rules.techs, EQUIPPED.1, Item::new(100, tech(EQUIPPED.0)?), false)
        .map_err(|_| format!("{}: not taken by slot {}", EQUIPPED.0, EQUIPPED.1))?;
    hovered.push((100, EQUIPPED.0, false));

    for (id, name, in_grid) in hovered {
        let t = tech(name)?;
        let class = rules
            .tech_class
            .get(&t)
            .ok_or_else(|| format!("{name}: no prefab class"))?;
        let path = class_paths
            .get(class)
            .ok_or_else(|| format!("{name}: class {class} not in prefabs.db"))?;
        let prefab = assets.prefab(&catalog, path)?;
        let values = assets.item_tooltip_values(&layouts, &prefab)?;
        println!("{name} ({path}): {values:?}");
        if !values.unported.is_empty() {
            println!("  not ported (lines left out): {:?}", values.unported);
        }
        let facts = values.facts(
            lang(name),
            lang(&format!("Tooltip_{name}")),
            t == inv::FIRST_AID_KIT,
        );
        let all = inv::item_actions(
            &rules.techs,
            &inventory,
            None,
            ItemId(id),
            values.eatable.is_some(),
            DropPlace::default(),
        );
        let actions = [
            inv::item_action(all, Button::Left),
            inv::item_action(all, Button::Right),
            0,
            0,
        ];
        // `hoveredItem == item && GetCanBindItem(item)`.
        let bind = in_grid && rules.techs.equipment_type(t) == hand;
        let (prefix, postfix) = inventory_item(&strings, &facts, true, actions, bind);
        println!("  prefix:  {prefix:?}");
        println!("  postfix: {postfix:?}");

        // The prefix, built here from the language file.
        let mut want = vec![title_line(&lang(name))];
        if let Some((charge, capacity)) = values.battery {
            let text = lang("BatteryCharge")
                .replace("{0:P0}", &format!("{}%", (100.0 * charge / capacity).round()))
                .replace("{1}", &format!("{}", charge.round()))
                .replace("{2}", &format!("{capacity}"));
            want.push(desc_line(&text));
        }
        if name == "FirstAidKit" {
            want.push(desc_line(&lang("HealthFormat").replace("{0:+#;-#;+0}", "+50")));
        }
        if let Some((food, water, _, _)) = values.eatable {
            for (v, key) in [(food, "FoodFormat"), (water, "WaterFormat")] {
                let v = v.ceil() as i64;
                if v != 0 {
                    want.push(desc_line(&lang(key).replace("{0:+#;-#;+0}", &signed(v))));
                }
            }
        }
        if values.battery.is_none() {
            want.push(desc_line(&lang(&format!("Tooltip_{name}"))));
        }
        let got: Vec<&str> = prefix.split('\n').collect();
        c.check(
            got == want.iter().map(String::as_str).collect::<Vec<_>>(),
            &format!("{name}: prefix {got:?}, expected {want:?}"),
        );
        // The value lines the rule names.
        let lines = got.len();
        let good = match name {
            "FilteredWater" => values.eatable.is_some_and(|e| e.1.ceil() != 0.0) && lines >= 3,
            "NutrientBlock" => values.eatable.is_some() && lines >= 3,
            "FirstAidKit" => lines == 3,
            "Battery" => values.battery.is_some() && lines == 2,
            _ => lines == 2,
        };
        c.check(good, &format!("{name}: {lines} prefix lines by the rule"));

        // The postfix: the rule's actions with the buttons' texts.
        let (want_bind, want_actions) = expected_actions(name);
        let mut want = Vec::new();
        if want_bind {
            want.push(action_line(&lang("KeyRange15"), &lang("BindQuickSlot")));
        }
        for &(b, key) in want_actions {
            want.push(action_line(&buttons[b], &lang(key)));
        }
        let got: Vec<&str> = if postfix.is_empty() {
            Vec::new()
        } else {
            postfix.split('\n').collect()
        };
        c.check(
            got == want.iter().map(String::as_str).collect::<Vec<_>>(),
            &format!("{name}: postfix {got:?}, expected {want:?}"),
        );

        check_layout(
            &mut c,
            name,
            &assets,
            &set,
            &mut scene,
            &config,
            &canvas,
            (&prefix, &postfix),
        )?;
    }

    println!("{} of {} checks passed", c.ok, c.all);
    if c.ok == c.all {
        println!("RUN OK");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("RUN FAILED");
        Ok(ExitCode::FAILURE)
    }
}
