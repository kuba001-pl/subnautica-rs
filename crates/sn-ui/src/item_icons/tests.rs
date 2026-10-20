use super::*;

const BG: Sprite = (SpriteId(1), [128.0, 128.0]);

fn item(id: u64, size: (i32, i32), at: (i32, i32)) -> TabItem {
    TabItem {
        id,
        size,
        at,
        background: BG,
        // 100 × 50: wider than tall.
        foreground: (SpriteId(100 + id), [100.0, 50.0]),
    }
}

fn maker() -> impl FnMut() -> IconNodes {
    let mut next = 10;
    move || {
        let i = IconNodes {
            root: next,
            background: next + 1,
            foreground: next + 2,
        };
        next += 3;
        i
    }
}

fn size(s: &UiState, n: usize) -> Option<V2> {
    s.transform.get(&n).and_then(|t| t.size_delta)
}

fn at(s: &UiState, n: usize) -> Option<V2> {
    s.transform.get(&n).and_then(|t| t.anchored_position)
}

#[test]
fn sizes_and_places_by_the_cells() {
    assert_eq!(icon_size((1, 1)), [66.0, 66.0]);
    assert_eq!(icon_size((2, 2)), [137.0, 137.0]);
    assert_eq!(icon_size((3, 1)), [208.0, 66.0]);
    assert_eq!(icon_position((0, 0)), [1.0, -1.0]);
    assert_eq!(icon_position((2, 3)), [143.0, -214.0]);
}

#[test]
fn an_added_item_gets_its_icon() {
    let mut c = ContainerIcons::default();
    let mut s = UiState::default();
    c.sync(&[item(7, (2, 2), (1, 0))], &mut s, &mut maker());
    let (icon, _) = c.icons[&7];
    assert_eq!(c.events.added, [7]);
    assert_eq!(c.events.made, 1);
    assert!(s.active[&icon.root]);
    assert_eq!(size(&s, icon.root), Some([137.0, 137.0]));
    assert_eq!(at(&s, icon.root), Some([72.0, -1.0]));
    // The background stretched, the foreground fitted (100 × 50 → 137 ×
    // 68.5).
    assert_eq!(size(&s, icon.background), Some([137.0, 137.0]));
    assert_eq!(size(&s, icon.foreground), Some([137.0, 68.5]));
    let m = &s.material[&icon.background];
    assert!(m.keywords["SLICE_9_GRID"]);
    assert_eq!(m.floats["_Radius"], 33.0);
    assert_eq!(m.vectors["_Size"], [137.0, 137.0, 0.0, 0.0]);
    assert_eq!(m.vectors["_FillRect"], [-68.5, -68.5, 137.0, 137.0]);
    assert_eq!(s.sprite[&icon.foreground], Some(SpriteId(107)));
    assert_eq!(s.sprite[&icon.background], Some(SpriteId(1)));
}

#[test]
fn moved_removed_and_reused() {
    let mut c = ContainerIcons::default();
    let mut s = UiState::default();
    let mut make = maker();
    c.sync(
        &[item(1, (1, 1), (0, 0)), item(2, (1, 1), (1, 0))],
        &mut s,
        &mut make,
    );
    let one = c.icons[&1].0;
    let two = c.icons[&2].0;
    // Item 1 moves, item 2 goes, item 3 comes and takes 2's nodes.
    c.sync(
        &[item(1, (1, 1), (5, 7)), item(3, (1, 1), (0, 0))],
        &mut s,
        &mut make,
    );
    assert_eq!(c.events.moved, [1]);
    assert_eq!(c.events.removed, [2]);
    assert_eq!(c.events.added, [3]);
    assert_eq!(c.events.made, 0);
    assert_eq!(at(&s, one.root), Some([356.0, -498.0]));
    assert_eq!(c.icons[&3].0, two);
    assert!(s.active[&two.root]);
    assert_eq!(s.sprite[&two.foreground], Some(SpriteId(103)));
    // Unchanged: nothing reported.
    c.sync(
        &[item(1, (1, 1), (5, 7)), item(3, (1, 1), (0, 0))],
        &mut s,
        &mut make,
    );
    assert_eq!(c.events, IconEvents::default());
    // `Uninit`.
    c.clear(&mut s);
    assert!(c.icons.is_empty());
    assert!(!s.active[&one.root]);
    assert!(!s.active[&two.root]);
}

#[test]
fn an_unplaced_item_hides_until_placed() {
    let mut c = ContainerIcons::default();
    let mut s = UiState::default();
    let mut make = maker();
    c.sync(&[item(1, (1, 1), (-1, -1))], &mut s, &mut make);
    let icon = c.icons[&1].0;
    assert!(!s.active[&icon.root]);
    c.sync(&[item(1, (1, 1), (2, 0))], &mut s, &mut make);
    assert!(s.active[&icon.root]);
}

#[test]
fn a_slot_shows_its_item_and_hides_its_hint() {
    let mut s = UiState::default();
    let icon = IconNodes {
        root: 1,
        background: 2,
        foreground: 3,
    };
    slot_set_item(
        &mut s,
        icon,
        Some(9),
        &item(4, (1, 1), (0, 0)),
        [80.0, 60.0],
    );
    assert!(s.active[&1]);
    assert!(!s.active[&9]);
    assert_eq!(size(&s, 1), Some([80.0, 60.0]));
    assert_eq!(s.material[&2].floats["_Radius"], 30.0);
    assert_eq!(at(&s, 1), Some([0.0, 0.0]));
    slot_clear(&mut s, Some(icon), Some(9));
    assert!(!s.active[&1]);
    assert!(s.active[&9]);
}
