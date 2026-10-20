use super::*;

/// Inventory 10 (grid 11), storage 20 (grid 21), torpedo 30 and 31,
/// equipment 40 with slots Head 41, Tank 42, SeamothModule1 43 (their
/// backgrounds + 100), labels 50 and 51. The containers' parents span
/// nothing (anchors one point), as stored.
fn config() -> InventoryTabConfig {
    let slot = |slot: &str, node: usize| SlotNodes {
        slot: slot.into(),
        node,
        background: node + 100,
        hint: None,
        icon_rect: node + 200,
        icon_size: [80.0, 80.0],
    };
    let mut language = BTreeMap::new();
    language.insert("InventoryLabel".into(), "INVENTORY".into());
    language.insert("StorageLabel".into(), "STORAGE".into());
    language.insert("EquipmentLabel".into(), "EQUIPMENT".into());
    InventoryTabConfig {
        inventory: ContainerNodes::new(10, Some(11)),
        storage: ContainerNodes::new(20, Some(21)),
        torpedo: vec![ContainerNodes::new(30, None), ContainerNodes::new(31, None)],
        equipment: 40,
        slots: vec![
            slot("Head", 41),
            slot("Tank", 42),
            slot("SeamothModule1", 43),
        ],
        inventory_label: 50,
        storage_label: 51,
        language,
    }
}

fn active(t: &InventoryTab, n: usize) -> Option<bool> {
    t.state.active.get(&n).copied()
}

#[test]
fn awake_hides_the_containers_the_doll_and_every_slot() {
    let t = InventoryTab::new(config());
    for n in [10, 20, 30, 31, 40, 41, 42, 43] {
        assert_eq!(active(&t, n), Some(false), "node {n}");
    }
    assert_eq!(t.state.color.get(&141), Some(&TINT_DEFAULT));
    assert_eq!(t.state.text[&50], "INVENTORY");
}

#[test]
fn start_sizes_the_inventory_by_its_cells() {
    let mut t = InventoryTab::new(config());
    t.start((6, 8));
    assert_eq!(active(&t, 10), Some(true));
    assert_eq!(t.state.transform[&10].size_delta, Some([426.0, 568.0]));
    assert_eq!(t.state.uv_rect[&11], [0.0, 0.0, 6.0, 8.0]);
    // A stretched parent: the size less its span.
    let mut c = config();
    c.inventory.parent_span = [100.0, 0.0];
    let mut t = InventoryTab::new(c);
    t.start((6, 8));
    assert_eq!(t.state.transform[&10].size_delta, Some([326.0, 568.0]));
}

#[test]
fn opening_without_storage_shows_the_players_slots_only() {
    let mut t = InventoryTab::new(config());
    t.start((6, 8));
    t.on_open_pda(UsedStorage::Equipment(vec![
        "Head".into(),
        "Tank".into(),
        "Nope".into(),
    ]));
    assert_eq!(active(&t, 40), Some(true));
    assert_eq!(active(&t, 41), Some(true));
    assert_eq!(active(&t, 42), Some(true));
    assert_eq!(active(&t, 43), Some(false));
    assert_eq!(active(&t, 20), Some(false), "no storage");
    assert_eq!(t.state.text[&51], "EQUIPMENT");
    t.on_close_pda();
    for n in [40, 41, 42, 43, 20] {
        assert_eq!(active(&t, n), Some(false), "node {n}");
    }
    assert_eq!(active(&t, 10), Some(true), "the inventory stays");
}

#[test]
fn opening_with_the_locker_shows_the_storage() {
    let mut t = InventoryTab::new(config());
    t.start((6, 8));
    t.on_open_pda(UsedStorage::Container {
        size: (4, 4),
        label: "LockerLabel".into(),
    });
    assert_eq!(active(&t, 20), Some(true));
    assert_eq!(t.state.transform[&20].size_delta, Some([284.0, 284.0]));
    assert_eq!(t.state.uv_rect[&21], [0.0, 0.0, 4.0, 4.0]);
    assert_eq!(active(&t, 40), Some(false), "the doll stays hidden");
    // A missing key reads as itself; an empty one as `StorageLabel`.
    assert_eq!(t.state.text[&51], "LockerLabel");
    t.on_close_pda();
    assert_eq!(active(&t, 20), Some(false));
    t.on_open_pda(UsedStorage::Container {
        size: (2, 2),
        label: String::new(),
    });
    assert_eq!(t.state.text[&51], "STORAGE");
}
