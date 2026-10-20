//! `ui --inventory` (M9u8b1, `docs/formats/ui.md` § The inventory tab):
//! the PDA's inventory tab without its icons, headless. `uGUI_InventoryTab`
//! read from the screen prefab, then its rules (`sn_ui::inventory_tab`)
//! with the PDA screen's (`sn_ui::pda`), laid out and drawn, checked
//! against values computed here:
//! - the `Awake`s: every container, the doll and every named slot hidden;
//! - `Start`: the inventory's rect 71 × the size the game's code gives
//!   `Inventory.Awake`'s container (6 × 8), its grid's `uvRect`;
//! - `OnOpenPDA` without storage: the doll shown with exactly the slots of
//!   `Inventory.UnlockDefaultEquipmentSlots` (written in DESIGN before the
//!   run: Head, Body, Gloves, Foots, Chip1, Chip2, Tank), every other slot
//!   hidden and drawing nothing; the storage label `EquipmentLabel`'s text;
//! - `OnOpenPDA` with the lifepod's locker: the storage's rect 71 × the
//!   locker's size, the doll hidden, the label the locker's;
//! - `OnClosePDA`: the storage and the doll hidden again.

use std::collections::{BTreeSet, HashMap};
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, UiScene, UiState};
use sn_install::GameData;
use sn_ui::inventory_tab::{CELL, InventoryTab, UsedStorage};
use sn_ui::layout::Tree;
use sn_ui::pda::PdaScreen;
use sn_ui::scaler::Placement;

use crate::Result;

/// The player's slots as DESIGN's M9u8b1 row wrote them before the run.
const EXPECTED_SLOTS: [&str; 7] = ["Head", "Body", "Gloves", "Foots", "Chip1", "Chip2", "Tank"];

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

fn below(tree: &Tree, mut n: usize, ancestor: usize) -> bool {
    for _ in 0..tree.nodes.len() {
        match tree.nodes[n].parent {
            Some(p) if p == ancestor => return true,
            Some(p) => n = p,
            None => return false,
        }
    }
    false
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let tags = sn_assets::physics_settings(&assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let code = sn_assets::read_assembly(game)?;
    let items = sn_assets::item_code(&code)?;
    let player = sn_assets::player_items(&assets)?;
    let locker = &player.pod_locker;
    let language = sn_assets::language(game, "English")?;
    let tree = assets.pda_screen_tree()?;
    let pda_config = assets.pda_screen_config(&layouts, &tree)?;
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let mut c = Checks { ok: 0, all: 0 };

    let mut config = assets.inventory_tab_config(
        &layouts,
        &tree,
        &language,
        &[locker.storage_label.as_str()],
    )?;
    println!(
        "uGUI_InventoryTab: inventory {} (grid {:?}), storage {} (grid {:?}), {} torpedo containers, equipment {} with {} named slots; labels {} / {}",
        tree.nodes[config.inventory.node].path,
        config.inventory.grid.map(|g| &tree.nodes[g].name),
        tree.nodes[config.storage.node].path,
        config.storage.grid.map(|g| &tree.nodes[g].name),
        config.torpedo.len(),
        tree.nodes[config.equipment].path,
        config.slots.len(),
        tree.nodes[config.inventory_label].name,
        tree.nodes[config.storage_label].name,
    );
    let known: BTreeSet<&str> = items.slot_types.iter().map(|(s, _)| s.as_str()).collect();
    let names: Vec<&str> = config.slots.iter().map(|s| s.slot.as_str()).collect();
    println!("  slots: {names:?}");
    let unknown: Vec<&&str> = names.iter().filter(|s| !known.contains(**s)).collect();
    c.check(
        unknown.is_empty(),
        &format!("every slot name is in Equipment.slotMapping: unknown {unknown:?}"),
    );
    let unique: BTreeSet<&str> = names.iter().copied().collect();
    c.check(unique.len() == names.len(), "slot names unique");
    println!(
        "  player's slots (Inventory.UnlockDefaultEquipmentSlots): {:?}; inventory {:?}; locker {}×{} label {:?}",
        items.player_slots, items.inventory_size, locker.width, locker.height, locker.storage_label
    );
    c.check(
        items.player_slots == EXPECTED_SLOTS,
        &format!("the player's slots as written: {:?}", items.player_slots),
    );
    let missing: Vec<&String> = items
        .player_slots
        .iter()
        .filter(|s| !names.contains(&s.as_str()))
        .collect();
    c.check(
        missing.is_empty(),
        &format!("every player slot has a uGUI_EquipmentSlot: missing {missing:?}"),
    );

    let stored = scene
        .stored_rect(pda_config.root)
        .cloned()
        .ok_or("the screen's root has no rect")?;
    let placements = HashMap::from([(
        pda_config.root,
        Placement {
            position: [0.0, 0.0, 0.19],
            rotation: [0.0, 0.0, 0.0, 1.0],
            size: stored.size_delta,
            scale: stored.scale,
            scale_factor: 1.0,
        },
    )]);
    let mut screen = PdaScreen::new(pda_config.clone());
    let state = |s: &PdaScreen, t: &InventoryTab| {
        let mut st = UiState::default();
        st.apply(&s.state);
        st.apply(&t.state);
        st
    };
    config.lay_out(&scene.layout_for(&assets, &set, &placements, &screen.state)?);
    let mut tab = InventoryTab::new(config.clone());
    let lay = |scene: &mut UiScene, st: &UiState| scene.layout_for(&assets, &set, &placements, st);

    // The `Awake`s.
    let l = lay(&mut scene, &state(&screen, &tab))?;
    let shown_slots = |l: &Tree| -> Vec<String> {
        config
            .slots
            .iter()
            .filter(|s| l.active(s.node))
            .map(|s| s.slot.clone())
            .collect()
    };
    c.check(!l.active(config.equipment), "Awake: the doll hidden");
    c.check(
        !l.active(config.inventory.node),
        "Awake: the inventory hidden",
    );
    c.check(shown_slots(&l).is_empty(), "Awake: no slot shown");

    // `Start`.
    let size = (items.inventory_size[0], items.inventory_size[1]);
    tab.start(size);
    screen.on_open_pda(None);
    tab.on_open_pda(UsedStorage::Equipment(items.player_slots.clone()));
    let st = state(&screen, &tab);
    let l = lay(&mut scene, &st)?;
    let r = l.rect(config.inventory.node);
    let want = [size.0 as f32 * CELL, size.1 as f32 * CELL];
    c.check(
        l.active(config.inventory.node)
            && (r.width - want[0]).abs() < 1e-3
            && (r.height - want[1]).abs() < 1e-3,
        &format!("the inventory's rect {r:?}, expected {want:?}, shown"),
    );
    if let Some(g) = config.inventory.grid {
        c.check(
            st.uv_rect.get(&g) == Some(&[0.0, 0.0, size.0 as f32, size.1 as f32]),
            "the inventory grid's uvRect",
        );
    }
    let shown = shown_slots(&l);
    println!("  open, no storage: slots shown {shown:?}");
    c.check(l.active(config.equipment), "open: the doll shown");
    let as_set = |v: &[String]| v.iter().cloned().collect::<BTreeSet<String>>();
    c.check(
        as_set(&shown) == as_set(&items.player_slots) && shown.len() == items.player_slots.len(),
        "open: exactly the player's slots shown (in hierarchy order)",
    );
    c.check(!l.active(config.storage.node), "open: no storage");
    c.check(
        st.text.get(&config.storage_label) == Some(&config.text("EquipmentLabel"))
            && st.text.get(&config.inventory_label) == Some(&config.text("InventoryLabel")),
        &format!(
            "labels {:?} / {:?}",
            st.text.get(&config.inventory_label),
            st.text.get(&config.storage_label)
        ),
    );
    println!(
        "  labels: {:?}, {:?}",
        st.text.get(&config.inventory_label),
        st.text.get(&config.storage_label)
    );
    // Drawn: nothing of a hidden slot, something of the doll.
    let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &st)?;
    let hidden: Vec<usize> = config
        .slots
        .iter()
        .filter(|s| !items.player_slots.contains(&s.slot))
        .map(|s| s.node)
        .collect();
    let in_hidden = d
        .list
        .items
        .iter()
        .filter(|it| {
            hidden
                .iter()
                .any(|&h| it.node == h || below(&l, it.node, h))
        })
        .count();
    let in_doll = d
        .list
        .items
        .iter()
        .filter(|it| below(&l, it.node, config.equipment))
        .count();
    let circles = d
        .list
        .items
        .iter()
        .filter(|it| below(&l, it.node, config.equipment))
        .filter(|it| {
            d.materials
                .get(it.material)
                .is_some_and(|m| m.shader == "UI/Circle")
        })
        .count();
    println!(
        "  drawn open: {} items, {in_doll} in the doll ({circles} with UI/Circle), {in_hidden} in hidden slots",
        d.list.items.len()
    );
    c.check(in_hidden == 0, "no draw of a hidden slot");
    c.check(in_doll > 0, "the doll draws");

    // With the locker.
    screen.on_close_pda();
    tab.on_close_pda();
    let l = lay(&mut scene, &state(&screen, &tab))?;
    c.check(!l.active(config.equipment), "closed: the doll hidden");
    c.check(shown_slots(&l).is_empty(), "closed: no slot shown");
    screen.on_open_pda(None);
    tab.on_open_pda(UsedStorage::Container {
        size: (locker.width, locker.height),
        label: locker.storage_label.clone(),
    });
    let st = state(&screen, &tab);
    let l = lay(&mut scene, &st)?;
    let r = l.rect(config.storage.node);
    let want = [locker.width as f32 * CELL, locker.height as f32 * CELL];
    c.check(
        l.active(config.storage.node)
            && (r.width - want[0]).abs() < 1e-3
            && (r.height - want[1]).abs() < 1e-3,
        &format!("the locker's rect {r:?}, expected {want:?}, shown"),
    );
    c.check(!l.active(config.equipment), "locker: the doll hidden");
    let label = config.text(&locker.storage_label);
    println!("  locker label: {label:?}");
    c.check(
        st.text.get(&config.storage_label) == Some(&label),
        "locker: its label",
    );
    screen.on_close_pda();
    tab.on_close_pda();
    let l = lay(&mut scene, &state(&screen, &tab))?;
    c.check(!l.active(config.storage.node), "closed: the storage hidden");
    c.check(
        l.active(config.inventory.node),
        "closed: the inventory stays",
    );

    println!("{} of {} checks ok", c.ok, c.all);
    if c.ok == c.all {
        println!("RUN OK");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("RUN FAILED");
        Ok(ExitCode::FAILURE)
    }
}
