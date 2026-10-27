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

use std::collections::HashMap;
use std::process::ExitCode;

use sn_assets::{Assets, UiState};
use sn_install::GameData;
use sn_sim::inventory::{self as inv, Button, DropPlace, Inventory, Item, ItemId, KnownTech};
use sn_ui::tooltip::{TooltipStrings, inventory_item};

use crate::Result;

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

/// Dumps the prefab's nodes under `uGUI/Tooltip`.
fn dump_prefab(game: &GameData, assets: &Assets) -> Result<()> {
    let tags = sn_assets::physics_settings(assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let catalog = assets.catalog()?;
    let root = assets.ugui_root(&catalog)?;
    let tree = assets.ui_tree("uGUI.prefab", &root)?;
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let config = assets.tooltip_config(&layouts, &tree)?;
    println!("uGUI_Tooltip: {config:?}");
    let laid = scene.layout_for(assets, &set, &HashMap::new(), &UiState::default())?;
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
    Ok(())
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    dump_prefab(game, &assets)?;
    let mut c = Checks { ok: 0, all: 0 };

    // The strings: the language file and the buttons.
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
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
    let catalog = assets.catalog()?;
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
