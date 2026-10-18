use super::*;

/// Nodes: 0 root (canvas group), 1 content, 2 toolbar group, 3 back
/// button, 4 background; tabs: intro object 10, inventory content 11,
/// journal content 12, ping 13, gallery 14, log 15, encyclopedia 16, time
/// capsule object 17.
fn config() -> PdaScreenConfig {
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
    PdaScreenConfig {
        root: 0,
        canvas_group: 0,
        content: 1,
        toolbar_group: 2,
        back_button: 3,
        background: 4,
        cycle_texts: vec![5, 6],
        tabs: tabs
            .iter()
            .map(|&(tab, show)| TabNodes { tab, show })
            .collect(),
    }
}

fn alpha(s: &PdaScreen, n: usize) -> Option<f32> {
    s.state.group_alpha.get(&n).copied()
}

#[test]
fn starts_hidden_with_every_tab_closed() {
    let s = PdaScreen::new(config());
    assert_eq!(alpha(&s, 0), Some(0.0), "canvas invisible");
    assert_eq!(alpha(&s, 1), Some(1.0), "content visible (Awake)");
    for n in 11..=16 {
        assert_eq!(alpha(&s, n), Some(0.0), "tab content {n}");
    }
    assert_eq!(s.state.active.get(&10), Some(&false));
    assert_eq!(s.state.active.get(&17), Some(&false));
    assert_eq!(s.state.active.get(&3), Some(&false));
    assert_eq!(s.state.active.get(&5), Some(&false), "gamepad label");
    assert_eq!(s.state.active.get(&6), Some(&false), "gamepad label");
    assert_eq!(s.state.color.get(&4).map(|c| c[3]), Some(1.0));
    assert_eq!(s.tab_open, None);
}

#[test]
fn opens_the_previous_tab_and_closes() {
    let mut s = PdaScreen::new(config());
    s.on_open_pda(None);
    assert_eq!(alpha(&s, 0), Some(1.0));
    assert_eq!(alpha(&s, 1), Some(1.0));
    assert_eq!(s.tab_open, Some(Tab::Inventory));
    assert_eq!(alpha(&s, 11), Some(1.0));
    assert_eq!(alpha(&s, 12), Some(0.0));
    assert_eq!(alpha(&s, 2), Some(1.0), "toolbar shown");
    // Another tab: the inventory closes, the log opens and becomes
    // `tabPrev`.
    assert!(s.open_tab(Tab::Log));
    assert!(!s.open_tab(Tab::Log), "already open");
    assert_eq!((alpha(&s, 11), alpha(&s, 15)), (Some(0.0), Some(1.0)));
    s.on_close_pda();
    assert_eq!(alpha(&s, 15), Some(0.0));
    assert_eq!(alpha(&s, 1), Some(0.0), "content hidden on close");
    assert_eq!(alpha(&s, 0), Some(1.0), "canvas until OnPDAClosed");
    s.on_pda_closed();
    assert_eq!(alpha(&s, 0), Some(0.0));
    // Opened again: the log, the last regular tab.
    s.on_open_pda(None);
    assert_eq!(s.tab_open, Some(Tab::Log));
    assert_eq!(alpha(&s, 1), Some(1.0));
}

#[test]
fn a_tab_off_the_toolbar_hides_it() {
    let mut s = PdaScreen::new(config());
    s.on_open_pda(Some(Tab::TimeCapsule));
    assert_eq!(s.state.active.get(&17), Some(&true));
    assert_eq!(alpha(&s, 2), Some(0.0));
    // Not a regular tab: `tabPrev` stays the inventory.
    assert_eq!(s.tab_prev, Tab::Inventory);
}

#[test]
fn shifted_moves_every_node() {
    let c = config().shifted(100);
    assert_eq!(c.root, 100);
    assert_eq!(c.tabs[1].show, TabShow::Content(111));
    assert_eq!(c.tabs[7].show, TabShow::Object(117));
}
