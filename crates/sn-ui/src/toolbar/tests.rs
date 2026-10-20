use super::*;
use crate::pda::{PdaScreen, PdaScreenConfig, REGULAR_TABS, Tab, TabNodes, TabShow};

const NORMAL: SpriteId = SpriteId(100);
const SELECTED: SpriteId = SpriteId(101);

/// The toolbar at node 20, its marker at 21; sizes as the game stores
/// them (background 143 × 54, foreground 64 × 64, button 127 × 54, step
/// −16, marker offset (−10, −35.5)); the marker anchored at its parent's
/// centre; the bar 60 high, its parent 0 wide.
fn config() -> ToolbarConfig {
    ToolbarConfig {
        node: 20,
        marker: Some(21),
        background_size: [143.0, 54.0],
        foreground_size: [64.0, 64.0],
        button_size: [127.0, 54.0],
        button_step: -16.0,
        marker_offset: [-10.0, -35.5],
        marker_anchor: [0.5, 0.5],
        height: 60.0,
        size_delta: [0.0, 60.0],
        parent_span_x: 0.0,
    }
}

/// Normal 160 × 54 (wider than the 143 × 54 box), selected 143 × 54.
fn sprites() -> ToolbarSprites {
    ToolbarSprites {
        normal: (NORMAL, [160.0, 54.0]),
        selected: (SELECTED, [143.0, 54.0]),
    }
}

/// Icons at 30 + 3i (root, background, foreground).
fn maker() -> impl FnMut() -> ToolbarIcon {
    let mut next = 30;
    move || {
        let i = ToolbarIcon {
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

/// Six tabs; tab 2 has no sprite; the others 128 × 96.
fn content() -> Vec<Option<IconSprite>> {
    (0..6)
        .map(|i| (i != 2).then_some((SpriteId(200 + i), [128.0, 96.0])))
        .collect()
}

#[test]
fn initialize_places_six_icons_by_the_rule() {
    let mut t = Toolbar::new(config(), sprites());
    let mut s = UiState::default();
    t.make_icons(6, &mut maker());
    t.initialize(&content(), &mut s);
    assert_eq!(t.count, 6);
    // 6 × 143 − 5 × 16 = 778.
    assert_eq!(t.width, 778.0);
    assert_eq!(size(&s, 20), Some([778.0, 60.0]));
    for i in 0..6 {
        let icon = t.icons[i];
        // x = 71.5 + 127 i, y = 27.
        assert_eq!(at(&s, icon.root), Some([71.5 + 127.0 * i as f32, 27.0]));
        assert_eq!(size(&s, icon.root), Some([127.0, 54.0]));
        assert_eq!(s.active.get(&icon.root), Some(&true));
        // The normal sprite kept its aspect inside 143 × 54: × 143/160.
        assert_eq!(s.sprite.get(&icon.background), Some(&Some(NORMAL)));
        let b = size(&s, icon.background).unwrap();
        assert!((b[0] - 143.0).abs() < 1e-4 && (b[1] - 54.0 * 143.0 / 160.0).abs() < 1e-4);
        assert_eq!(
            s.material[&icon.background].vectors["_FillRect"],
            [-71.5, -32.0, 143.0, 64.0]
        );
        assert_eq!(
            s.material[&icon.background].vectors["_Size"],
            [143.0, 54.0, 0.0, 0.0]
        );
        if i == 2 {
            assert_eq!(s.enabled.get(&icon.foreground), Some(&false));
        } else {
            assert_eq!(
                s.sprite.get(&icon.foreground),
                Some(&Some(SpriteId(200 + i as u64)))
            );
            // 128 × 96 into 64 × 64: 64 × 48.
            assert_eq!(size(&s, icon.foreground), Some([64.0, 48.0]));
        }
    }
    assert_eq!(t.selected, None);
    assert_eq!(s.active.get(&21), Some(&false), "marker hidden");
    assert_eq!(t.events.last_sibling, [21]);
}

#[test]
fn select_swaps_backgrounds_and_moves_the_marker() {
    let mut t = Toolbar::new(config(), sprites());
    let mut s = UiState::default();
    t.make_icons(6, &mut maker());
    t.initialize(&content(), &mut s);
    assert!(t.select(1, &mut s));
    let (a, b) = (t.icons[0], t.icons[1]);
    assert_eq!(s.sprite.get(&b.background), Some(&Some(SELECTED)));
    // Marker: the icon's (198.5, 27) + (−10, −35.5) less the anchor
    // reference (778 × ½, 60 × ½).
    assert_eq!(at(&s, 21), Some([198.5 - 10.0 - 389.0, 27.0 - 35.5 - 30.0]));
    assert_eq!(s.active.get(&21), Some(&true));
    assert!(t.select(3, &mut s));
    // Back to normal, now through `SetBackgroundSprite`'s own size call:
    // the aspect not kept, 143 × 54 exactly.
    assert_eq!(s.sprite.get(&b.background), Some(&Some(NORMAL)));
    assert_eq!(size(&s, b.background), Some([143.0, 54.0]));
    // An icon never selected keeps the first fit.
    assert!(size(&s, a.background).unwrap()[1] < 54.0);
    assert!(!t.select(6, &mut s), "out of range");
    assert_eq!(t.selected, Some(3));
}

#[test]
fn a_click_hits_the_icon_under_it_the_later_one_on_a_border() {
    let mut t = Toolbar::new(config(), sprites());
    let mut s = UiState::default();
    t.make_icons(6, &mut maker());
    t.initialize(&content(), &mut s);
    // The toolbar's rect with its pivot at its centre.
    let r = Rect {
        x: -389.0,
        y: -30.0,
        width: 778.0,
        height: 60.0,
    };
    // Icon 0's button spans x 8…135 from the left edge, y 0…54.
    assert_eq!(t.hit([-389.0 + 70.0, -30.0 + 27.0], r), Some(0));
    assert_eq!(t.hit([-389.0 + 135.0, -30.0 + 27.0], r), Some(1));
    assert_eq!(t.hit([-389.0 + 5.0, -30.0 + 27.0], r), None);
    assert_eq!(t.hit([-389.0 + 70.0, -30.0 + 55.0], r), None);
    assert_eq!(
        t.hit([-389.0 + 71.5 + 5.0 * 127.0, -30.0 + 1.0], r),
        Some(5)
    );
}

fn screen() -> PdaScreen {
    let tabs = [
        (Tab::Intro, TabShow::Object(10)),
        (Tab::Inventory, TabShow::Content(11)),
        (Tab::Journal, TabShow::Content(12)),
        (Tab::Ping, TabShow::Content(13)),
        (Tab::Gallery, TabShow::Content(14)),
        (Tab::Log, TabShow::Content(15)),
        (Tab::Encyclopedia, TabShow::Content(16)),
        (Tab::TimeCapsule, TabShow::Object(17)),
    ];
    let mut p = PdaScreen::new(PdaScreenConfig {
        root: 0,
        canvas_group: 0,
        content: 1,
        toolbar_group: 2,
        back_button: 3,
        background: 4,
        cycle_texts: vec![],
        tabs: tabs
            .iter()
            .map(|&(tab, show)| TabNodes { tab, show })
            .collect(),
    });
    let sprites = REGULAR_TABS
        .iter()
        .enumerate()
        .map(|(i, &t)| (t, (SpriteId(200 + i as u64), [64.0, 64.0])))
        .collect();
    p.attach_toolbar(
        Toolbar::new(config(), self::sprites()),
        sprites,
        &mut maker(),
    );
    p
}

#[test]
fn the_pda_selects_the_open_tab_and_a_click_switches() {
    let mut p = screen();
    let made = p.toolbar.as_ref().unwrap().icons.clone();
    assert_eq!(made.len(), 6);
    p.on_open_pda(None);
    assert_eq!(p.toolbar.as_ref().unwrap().selected, Some(0), "Inventory");
    assert_eq!(p.state.sprite[&made[0].background], Some(SELECTED));
    // A left click on icon 4 (Log): opened and selected.
    assert!(p.on_toolbar_click(4, 0));
    assert_eq!(p.tab_open, Some(Tab::Log));
    assert_eq!(p.state.group_alpha[&11], 0.0);
    assert_eq!(p.state.group_alpha[&15], 1.0);
    assert_eq!(p.state.sprite[&made[0].background], Some(NORMAL));
    assert_eq!(p.state.sprite[&made[4].background], Some(SELECTED));
    // The right button, the open tab and an index past the end do nothing.
    assert!(!p.on_toolbar_click(2, 1));
    assert!(!p.on_toolbar_click(4, 0));
    assert!(!p.on_toolbar_click(6, 0));
    // Closing re-makes the bar: nothing selected, the marker hidden, the
    // same nodes; the next open opens the previous tab (Log).
    p.on_close_pda();
    let tb = p.toolbar.as_ref().unwrap();
    assert_eq!(tb.selected, None);
    assert_eq!(tb.icons, made);
    assert_eq!(p.state.sprite[&made[4].background], Some(NORMAL));
    assert!(!p.state.active[&21]);
    p.on_open_pda(None);
    assert_eq!(p.tab_open, Some(Tab::Log));
    assert_eq!(p.toolbar.as_ref().unwrap().selected, Some(4));
}
