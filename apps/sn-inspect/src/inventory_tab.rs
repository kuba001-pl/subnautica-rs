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
//!
//! The icons (M9u8b2a): a scripted set of items (the grid, the player's
//! slots, the locker; 1 × 1 and larger) through `sync_items`; each icon's
//! laid-out rect against the rule (size 66·w + 5·(w − 1), top left 71·x + 1
//! from the container's; a slot's icon centred on and sized to its
//! `iconRect`), two draws per icon (its background with `SLICE_9_GRID`
//! and its foreground); then a move, a removal and an unequip, and the
//! close (the locker's icons and the slots' cleared).
//!
//! The pointer (M9u8b3a): a hit at each icon's and each filled slot's
//! centre finds its item, at an empty cell nothing; hovering the equipped
//! `Tank` tints exactly the slots of its `EquipmentType` (written in
//! DESIGN before the run: the Tank slot only), `Titanium` none; nothing is
//! drawn over an icon's centre after the icons.
//!
//! Dragging (M9u8b4): the Tank and the Fins dragged over each shown slot
//! tint the slots by `UpdateFeedbackState` (written before the run: the
//! compatible slot allowed when hovered, else highlighted; another hovered
//! slot denied, the rest default); an empty slot's snap lands on its icon
//! with its radius; a locked icon draws at 0.3; `ItemActions/Swap` exists.

use std::collections::{BTreeSet, HashMap};
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, UiScene, UiState};
use sn_install::GameData;
use sn_ui::drag::{LOCK_ALPHA, Snap, TINT_ALLOWED, TINT_DENIED, slot_tint};
use sn_ui::draw::{apply, to_ancestor};
use sn_ui::inventory_tab::{
    CELL, InventoryTab, MakeIcon, TINT_DEFAULT, TINT_HIGHLIGHTED, TabHit, UsedStorage,
};
use sn_ui::item_icons::{ContainerIcons, IconNodes, Sprite, TabItem, icon_position, icon_size};
use sn_ui::layout::{Tree, V2};
use sn_ui::pda::PdaScreen;
use sn_ui::scaler::Placement;

use crate::Result;

/// The player's slots as DESIGN's M9u8b1 row wrote them before the run.
const EXPECTED_SLOTS: [&str; 7] = ["Head", "Body", "Gloves", "Foots", "Chip1", "Chip2", "Tank"];
/// The scripted items: the grid's, the locker's, the slots'.
const GRID: [&str; 4] = ["Titanium", "ScrapMetal", "Battery", "FilteredWater"];
const LOCKER: [&str; 3] = ["Flare", "NutrientBlock", "FirstAidKit"];
const EQUIPPED: [(&str, &str); 3] = [("Head", "Rebreather"), ("Tank", "Tank"), ("Foots", "Fins")];

/// An item's size and sprite objects, by its tech type's name.
type ItemLook = (
    (i32, i32),
    (sn_assets::ObjectRef, V2),
    (sn_assets::ObjectRef, V2),
);

/// Our items for `names`, laid left to right on row 0 (`first` the first
/// id), their sprites given ids by `scene`.
fn items_of(
    scene: &mut UiScene,
    names: &[&str],
    first: u64,
    look: &dyn Fn(&str) -> Result<ItemLook>,
) -> Result<Vec<TabItem>> {
    let mut out = Vec::new();
    let mut x = 0;
    for (k, n) in names.iter().enumerate() {
        let (size, (bg, bs), (fg, fs)) = look(n)?;
        let background: Sprite = (scene.sprite_id(&bg), bs);
        let foreground: Sprite = (scene.sprite_id(&fg), fs);
        out.push(TabItem {
            id: first + k as u64,
            size,
            at: (x, 0),
            background,
            foreground,
        });
        x += size.0;
    }
    Ok(out)
}

/// Every icon of `icons` (with its item) against the rule in the laid-out
/// tree `l`: the container `parent`'s top left + the place, the size.
fn check_container_icons(
    c: &mut Checks,
    l: &Tree,
    parent: usize,
    icons: &ContainerIcons,
    items: &[TabItem],
    what: &str,
) {
    let pr = l.rect(parent);
    for it in items {
        let Some((icon, _)) = icons.icons.get(&it.id) else {
            c.check(false, &format!("{what}: item {} has no icon", it.id));
            continue;
        };
        let want_size = icon_size(it.size);
        let p = icon_position(it.at);
        // Pivot (0, 1) at the anchor (0, 1): the parent's top left.
        let want_at = [pr.x + p[0], pr.y + pr.height + p[1]];
        let got_at = l.local_position(icon.root);
        let got_size = l.rect(icon.root).size();
        let good = (got_at[0] - want_at[0]).abs() < 1e-3
            && (got_at[1] - want_at[1]).abs() < 1e-3
            && (got_size[0] - want_size[0]).abs() < 1e-3
            && (got_size[1] - want_size[1]).abs() < 1e-3
            && l.active(icon.root);
        c.check(
            good,
            &format!(
                "{what}: item {} {:?} at {:?}: icon {got_size:?} at {got_at:?}, expected {want_size:?} at {want_at:?}",
                it.id, it.size, it.at
            ),
        );
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

    // The icons: the grid and the slots.
    let gc = sn_assets::game_code(&code)?;
    let td = sn_assets::tech_data(&assets)?;
    let catalog = assets.catalog()?;
    let sprites = assets.sprites(&catalog)?;
    let backgrounds = assets.background_sprites()?;
    for (k, b) in backgrounds.0.iter().enumerate() {
        println!(
            "  background {k} ({}): {:?}",
            sn_assets::BACKGROUND_PATHS[k],
            b.as_ref().map(|b| (&b.name, b.size, &b.object.file.name))
        );
    }
    c.check(
        backgrounds.0.iter().all(Option::is_some),
        "every background sprite found in Resources",
    );
    let material = assets.icon_material(&catalog, &layouts)?;
    let layer = tree.nodes[config.inventory.node].layer;
    let item_template = UiScene::from_code(
        "Item Icon",
        sn_assets::item_icon_nodes(layer, &material, false),
    )?;
    let slot_template = UiScene::from_code(
        "Equipment Slot Icon",
        sn_assets::item_icon_nodes(layer, &material, true),
    )?;
    let look = |name: &str| -> Result<ItemLook> {
        let tech = gc
            .tech_types
            .iter()
            .find(|(_, n)| n == name)
            .map(|(v, _)| *v)
            .ok_or_else(|| format!("no tech type {name}"))?;
        let (size, bt) = sn_assets::tech_icon(&td, &gc.tech_defaults, tech);
        let fg = sprites
            .item_icon(name)
            .ok_or_else(|| format!("{name}: no sprite"))?;
        let bg = backgrounds
            .get(bt)
            .ok_or_else(|| format!("{name}: background {bt} not found"))?;
        Ok((
            size,
            (bg.object.clone(), bg.size),
            (fg.object.clone(), [fg.sprite.rect[2], fg.sprite.rect[3]]),
        ))
    };
    let grid = items_of(&mut scene, &GRID, 100, &look)?;
    let slot_items: Vec<&str> = EQUIPPED.iter().map(|(_, n)| *n).collect();
    let equipped = items_of(&mut scene, &slot_items, 200, &look)?;
    let equipment: Vec<(String, TabItem)> = EQUIPPED
        .iter()
        .zip(&equipped)
        .map(|((s, _), it)| (s.to_string(), *it))
        .collect();
    for it in grid.iter().chain(&equipped) {
        println!("  item {}: {:?} cells at {:?}", it.id, it.size, it.at);
    }
    let mut made = 0usize;
    let mut graft_failed = None;
    let mut make = |scene: &mut UiScene, m: MakeIcon| {
        made += 1;
        let (t, parent) = match m {
            MakeIcon::Item(p) => (&item_template, p),
            MakeIcon::Slot(p) => (&slot_template, p),
        };
        match scene.graft(t, parent) {
            Ok(n) => IconNodes {
                root: n,
                background: n + 1,
                foreground: n + 2,
            },
            Err(e) => {
                graft_failed = Some(e);
                IconNodes {
                    root: usize::MAX,
                    background: usize::MAX,
                    foreground: usize::MAX,
                }
            }
        }
    };
    tab.sync_items(&grid, &[], &equipment, &mut |m| make(&mut scene, m));
    let st = state(&screen, &tab);
    let l = lay(&mut scene, &st)?;
    check_container_icons(
        &mut c,
        &l,
        config.inventory.node,
        &tab.inventory_icons,
        &grid,
        "grid",
    );
    for (slot, it) in &equipment {
        let Some(sc) = config.slots.iter().find(|s| &s.slot == slot) else {
            c.check(false, &format!("no slot {slot}"));
            continue;
        };
        let Some(si) = tab.slot_icons.get(slot) else {
            c.check(false, &format!("{slot}: no icon"));
            continue;
        };
        let r = l.rect(sc.icon_rect);
        let at = l.local_position(si.icon.root);
        let size = l.rect(si.icon.root).size();
        let good = si.item == Some(it.id)
            && l.active(si.icon.root)
            && (at[0] - (r.x + 0.5 * r.width)).abs() < 1e-3
            && (at[1] - (r.y + 0.5 * r.height)).abs() < 1e-3
            && (size[0] - r.width).abs() < 1e-3
            && (size[1] - r.height).abs() < 1e-3
            && sc.hint.is_none_or(|h| !l.active(h));
        println!(
            "  slot {slot}: iconRect {:?}, icon {size:?}, radius {:?}, hint {:?}",
            r.size(),
            st.material
                .get(&si.icon.background)
                .and_then(|m| m.floats.get("_Radius")),
            sc.hint.map(|h| &tree.nodes[h].name)
        );
        c.check(
            good,
            &format!("{slot}'s icon centred on and sized to its iconRect, hint hidden"),
        );
    }
    let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &st)?;
    let icon_nodes: Vec<IconNodes> = tab
        .inventory_icons
        .icons
        .values()
        .map(|(i, _)| *i)
        .chain(tab.slot_icons.values().map(|s| s.icon))
        .collect();
    let (mut drawn, mut slice9) = (0, 0);
    for it in &d.list.items {
        if let Some(icon) = icon_nodes
            .iter()
            .find(|i| i.background == it.node || i.foreground == it.node)
        {
            drawn += 1;
            let m = d.materials.get(it.material);
            if it.node == icon.background
                && m.is_some_and(|m| m.keywords.iter().any(|k| k == "SLICE_9_GRID"))
            {
                slice9 += 1;
            }
        }
    }
    println!(
        "  drawn: {drawn} icon graphics for {} icons, {slice9} backgrounds with SLICE_9_GRID",
        icon_nodes.len()
    );
    c.check(drawn == 2 * icon_nodes.len(), "two draws per icon");
    c.check(
        slice9 == icon_nodes.len(),
        "every background with SLICE_9_GRID",
    );
    // The pointer: hits at the icons' and slots' centres.
    let root = pda_config.root;
    let centre = |l: &Tree, n: usize| {
        let r = l.rect(n);
        let q = apply(
            &to_ancestor(l, n, root),
            [r.x + 0.5 * r.width, r.y + 0.5 * r.height, 0.0],
        );
        [q[0], q[1]]
    };
    let mut centres = Vec::new();
    for (id, (icon, _)) in &tab.inventory_icons.icons {
        let p = centre(&l, icon.root);
        centres.push(p);
        let hit = tab.hit(&l, root, p);
        c.check(
            hit == Some(TabHit::Item(*id)),
            &format!("hit at item {id}'s centre: {hit:?}"),
        );
    }
    for (slot, si) in &tab.slot_icons {
        let Some(sc) = config.slots.iter().find(|s| &s.slot == slot) else {
            continue;
        };
        let p = centre(&l, sc.node);
        centres.push(p);
        let hit = tab.hit(&l, root, p);
        c.check(
            hit.as_ref().and_then(TabHit::item) == si.item,
            &format!("hit at slot {slot}'s centre: {hit:?}"),
        );
    }
    // An empty cell: the last of the grid.
    let inv = l.rect(config.inventory.node);
    let empty = apply(
        &to_ancestor(&l, config.inventory.node, root),
        [inv.x + inv.width - 0.5 * CELL, inv.y + 0.5 * CELL, 0.0],
    );
    let hit = tab.hit(&l, root, [empty[0], empty[1]]);
    c.check(hit.is_none(), &format!("hit at an empty cell: {hit:?}"));
    // Nothing drawn over an icon's centre after the icons.
    let last_icon = d
        .list
        .items
        .iter()
        .rposition(|it| {
            icon_nodes
                .iter()
                .any(|i| i.background == it.node || i.foreground == it.node)
        })
        .unwrap_or(0);
    let over: Vec<String> = d
        .list
        .items
        .iter()
        .skip(last_icon + 1)
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
    println!("  drawn after the icons over a centre: {over:?}");
    c.check(over.is_empty(), "nothing drawn over the icons");
    // Hover: the compatible slots (`Equipment.GetSlots`: the player's
    // slots whose type is the item's `EquipmentType`).
    let slot_type = |s: &str| {
        items
            .slot_types
            .iter()
            .find(|(n, _)| n == s)
            .map(|(_, t)| *t)
    };
    let compatible = |name: &str| -> Vec<String> {
        let tech = gc
            .tech_types
            .iter()
            .find(|(_, n)| n == name)
            .map(|(v, _)| *v);
        let et = tech
            .and_then(|t| td.get(t))
            .and_then(|e| e.equipment_type)
            .unwrap_or(gc.tech_defaults.equipment_type);
        items
            .player_slots
            .iter()
            .filter(|s| slot_type(s) == Some(et))
            .cloned()
            .collect()
    };
    let lit = |tab: &InventoryTab| -> Vec<String> {
        config
            .slots
            .iter()
            .filter(|s| tab.state.color.get(&s.background) == Some(&TINT_HIGHLIGHTED))
            .map(|s| s.slot.clone())
            .collect()
    };
    let tank = compatible("Tank");
    tab.set_hovered(Some((201, &tank)));
    println!("  hover Tank: compatible {tank:?}, lit {:?}", lit(&tab));
    c.check(
        lit(&tab) == ["Tank"],
        "hovering the Tank lights the Tank slot only",
    );
    let titanium = compatible("Titanium");
    tab.set_hovered(Some((100, &titanium)));
    println!(
        "  hover Titanium: compatible {titanium:?}, lit {:?}",
        lit(&tab)
    );
    c.check(lit(&tab).is_empty(), "hovering Titanium lights nothing");
    tab.set_hovered(None);

    // Dragging (M9u8b4): the slots' feedback while the Tank and the Fins
    // are dragged over each shown slot, against the rule written before the
    // run (the compatible slot allowed when hovered, highlighted when not;
    // any other hovered slot denied, else default).
    for (name, want) in [("Tank", "Tank"), ("Fins", "Foots")] {
        let comp = compatible(name);
        c.check(comp == [want], &format!("{name}: compatible {comp:?}"));
        for over in shown_slots(&l) {
            let mut bad = Vec::new();
            for s in shown_slots(&l) {
                let tint = slot_tint(comp.contains(&s), s == over, true);
                let rule = match (s == want, s == over) {
                    (true, true) => TINT_ALLOWED,
                    (true, false) => TINT_HIGHLIGHTED,
                    (false, true) => TINT_DENIED,
                    (false, false) => TINT_DEFAULT,
                };
                if tint != rule {
                    bad.push(s);
                }
            }
            c.check(
                bad.is_empty(),
                &format!("dragging {name} over {over}: wrong tints {bad:?}"),
            );
        }
    }
    // An empty slot's snap: its `iconRect`'s centre and size, the radius a
    // slot icon gets (`GetBackgroundRadius`).
    let st = state(&screen, &tab);
    for (slot, si) in &tab.slot_icons {
        let Some(sc) = config.slots.iter().find(|s| &s.slot == slot) else {
            continue;
        };
        let snap = Snap::slot(centre(&l, sc.icon_rect), l.rect(sc.icon_rect).size());
        let radius = st
            .material
            .get(&si.icon.background)
            .and_then(|m| m.floats.get("_Radius"))
            .copied();
        let at = centre(&l, si.icon.root);
        println!(
            "  drag snap {slot}: at {:?}, size {:?}, radius {} (the slot icon's {radius:?})",
            snap.at, snap.size, snap.radius
        );
        c.check(
            radius == Some(snap.radius)
                && (snap.at[0] - at[0]).abs() < 1e-3
                && (snap.at[1] - at[1]).abs() < 1e-3,
            &format!("{slot}: the snap on the slot's icon, its radius"),
        );
    }
    // The locked icon (`OnLock`): its two graphics drawn at 0.3 of their
    // alpha.
    let (first, _) = tab
        .inventory_icons
        .icons
        .values()
        .next()
        .copied()
        .ok_or("no grid icon")?;
    let alpha_of = |d: &sn_assets::UiDraw| -> Vec<f32> {
        d.list
            .items
            .iter()
            .filter(|it| it.node == first.background || it.node == first.foreground)
            .map(|it| it.alpha)
            .collect()
    };
    let before = alpha_of(&scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &st)?);
    let mut locked = st.clone();
    locked.renderer_alpha.insert(first.background, LOCK_ALPHA);
    locked.renderer_alpha.insert(first.foreground, LOCK_ALPHA);
    let after = alpha_of(&scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &locked)?);
    println!("  locked icon: alpha {before:?} → {after:?}");
    c.check(
        before.len() == 2
            && after.len() == 2
            && before
                .iter()
                .zip(&after)
                .all(|(b, a)| (a - b * LOCK_ALPHA).abs() < 1e-5),
        "the locked icon draws at 0.3",
    );
    // The swap hint's sprite.
    let swap = sprites.get_no_default(sn_assets::GROUP_ITEM_ACTIONS, "Swap");
    println!(
        "  ItemActions/Swap: {:?}",
        swap.map(|s| (s.sprite.rect, s.sprite.pixels_to_units))
    );
    c.check(swap.is_some(), "ItemActions/Swap found");

    // A move and a removal in the grid, then an unequip.
    let mut grid2 = grid.clone();
    grid2[0].at = (5, 7);
    grid2.remove(1);
    tab.sync_items(&grid2, &[], &equipment, &mut |m| make(&mut scene, m));
    let ev = tab.inventory_icons.events.clone();
    c.check(
        ev.moved == [grid[0].id] && ev.removed == [grid[1].id] && ev.added.is_empty(),
        &format!("move and removal: {ev:?}"),
    );
    let l = lay(&mut scene, &state(&screen, &tab))?;
    check_container_icons(
        &mut c,
        &l,
        config.inventory.node,
        &tab.inventory_icons,
        &grid2,
        "grid after",
    );
    tab.sync_items(&grid2, &[], &equipment[1..], &mut |m| make(&mut scene, m));
    let st = state(&screen, &tab);
    let head = &tab.slot_icons["Head"];
    c.check(
        head.item.is_none() && st.active.get(&head.icon.root) == Some(&false),
        "unequipped: the Head icon hidden",
    );

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
    c.check(
        tab.slot_icons.values().all(|s| s.item.is_none()),
        "the slots were cleared at the close",
    );
    let stored = items_of(&mut scene, &LOCKER, 300, &look)?;
    tab.sync_items(&grid2, &stored, &[], &mut |m| make(&mut scene, m));
    let l = lay(&mut scene, &state(&screen, &tab))?;
    check_container_icons(
        &mut c,
        &l,
        config.storage.node,
        &tab.storage_icons,
        &stored,
        "locker",
    );
    screen.on_close_pda();
    tab.on_close_pda();
    let l = lay(&mut scene, &state(&screen, &tab))?;
    c.check(!l.active(config.storage.node), "closed: the storage hidden");
    c.check(
        tab.storage_icons.icons.is_empty(),
        "closed: the locker's icons gone",
    );
    if let Some(e) = graft_failed {
        return Err(format!("graft: {e}"));
    }
    println!("  icons made by code: {made}");
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
