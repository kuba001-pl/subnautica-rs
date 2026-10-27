use super::*;
use crate::state::SpriteId;

fn craft(id: &str, tech: i32) -> SourceNode {
    SourceNode {
        id: id.into(),
        action: NodeAction::Craft,
        tech,
        children: Vec::new(),
    }
}

fn group(id: &str, children: Vec<SourceNode>) -> SourceNode {
    SourceNode {
        id: id.into(),
        action: NodeAction::Expand,
        tech: 0,
        children,
    }
}

/// Root → Resources (1, 2, 3, 4, 5), Survival (6), Submarine (7),
/// Empty (8: hidden), Tools (9: locked).
fn tree() -> SourceNode {
    SourceNode {
        id: "Root".into(),
        action: NodeAction::None,
        tech: 0,
        children: vec![
            group(
                "Resources",
                vec![
                    craft("A", 1),
                    craft("B", 2),
                    craft("C", 3),
                    craft("D", 4),
                    craft("E", 5),
                ],
            ),
            group("Survival", vec![craft("F", 6)]),
            group("Submarine", vec![craft("G", 7)]),
            group("Empty", vec![craft("H", 8)]),
            group("Tools", vec![craft("I", 9)]),
        ],
    }
}

fn unlock(t: i32) -> Unlock {
    match t {
        8 => Unlock::Hidden,
        9 => Unlock::Locked,
        _ => Unlock::Available,
    }
}

fn yes(_: i32) -> bool {
    true
}

/// Only 1 and 9 have their ingredients.
fn fulfilled(t: i32) -> bool {
    t == 1 || t == 9
}

fn world() -> MenuWorld<'static> {
    MenuWorld {
        survival: true,
        blueprints: true,
        in_submarine: false,
        unlock: &unlock,
        has_recipe: &yes,
        fulfilled: &fulfilled,
    }
}

fn paths(nodes: &[Node]) -> Vec<&str> {
    nodes.iter().map(|n| n.path.as_str()).collect()
}

#[test]
fn the_filter_drops_hidden_recipes_empty_and_filtered_categories() {
    let n = filter(&tree(), &world()).unwrap();
    assert_eq!(
        paths(&n),
        [
            "",
            "Resources",
            "Resources/A",
            "Resources/B",
            "Resources/C",
            "Resources/D",
            "Resources/E",
            "Survival",
            "Survival/F",
            "Tools",
            "Tools/I"
        ]
    );
    assert_eq!(n[0].children, [1, 7, 9]);
    assert_eq!(n[1].children, [2, 3, 4, 5, 6]);
    assert_eq!((n[2].depth, n[2].parent), (2, Some(1)));
    // Outside survival the category goes; in a submarine its one comes.
    let w = MenuWorld {
        survival: false,
        in_submarine: true,
        ..world()
    };
    let n = filter(&tree(), &w).unwrap();
    assert!(!paths(&n).contains(&"Survival"));
    assert!(paths(&n).contains(&"Submarine/G"));
    // Without blueprints every recipe shows.
    let w = MenuWorld {
        blueprints: false,
        ..world()
    };
    assert!(paths(&filter(&tree(), &w).unwrap()).contains(&"Empty/H"));
}

#[test]
fn metrics_follow_get_icon_metrics() {
    // A column: x from the parent's right edge, rows centred.
    assert_eq!(icon_metrics(0.0, 1, 0, 3, false), (92.0, 46.0, 92.0));
    assert_eq!(icon_metrics(0.0, 1, 2, 3, false), (92.0, 46.0, -92.0));
    // A grid of 5: 3 columns, 2 rows; under a 92 wide parent.
    let (s, x, y) = icon_metrics(92.0, 2, 4, 5, true);
    assert!((s - 92.0 / 1.18).abs() < 1e-4);
    assert!((x - (1.5 * s + 46.0)).abs() < 1e-4);
    assert!((y + 0.5 * s).abs() < 1e-4);
    // The size stops at 40.
    assert_eq!(icon_metrics(0.0, 9, 0, 1, true).0, 40.0);
    assert!((icon_metrics(0.0, 5, 0, 1, true).0 - 92.0 / 1.18f32.powi(4)).abs() < 1e-4);
}

fn sprite(n: u64) -> Option<Sprite> {
    Some((SpriteId(n), [100.0, 100.0]))
}

fn config() -> CraftingMenuConfig {
    CraftingMenuConfig {
        icons_canvas: 1000,
        canvas_group: 1001,
        expand_normal: sprite(1),
        expand_hovered: sprite(2),
        craft_normal: sprite(3),
        craft_hovered: sprite(4),
    }
}

struct Maker(usize, Vec<usize>);

impl Maker {
    fn make(&mut self, parent: usize) -> IconNodes {
        self.1.push(parent);
        let n = self.0;
        self.0 += 3;
        IconNodes {
            root: n,
            background: n + 1,
            foreground: n + 2,
        }
    }
}

fn fg(_: &Node) -> Option<Sprite> {
    Some((SpriteId(9), [50.0, 25.0]))
}

fn opened() -> (CraftingMenu, Maker) {
    let mut m = CraftingMenu::new(config(), 7);
    let mut mk = Maker(0, Vec::new());
    assert!(m.open(
        &tree(),
        &world(),
        false,
        0.0,
        0.016,
        &mut |p| mk.make(p),
        &fg
    ));
    (m, mk)
}

#[test]
fn open_shows_the_top_level_in_a_column_and_punches_it() {
    let (m, mk) = opened();
    assert_eq!(mk.1, [1000, 1000, 1000]);
    let placed = m.placed();
    assert_eq!(placed.len(), 3);
    assert_eq!(placed[0], ("Resources".into(), [46.0, 92.0], 92.0));
    assert_eq!(placed[2], ("Tools".into(), [46.0, -92.0], 92.0));
    // The foreground: 0.674 × 92 fitted keeping the sprite's 2:1 aspect.
    let fore = m.icons["Resources"].nodes.foreground;
    let s = m.state.transform[&fore].size_delta.unwrap();
    assert!((s[0] - 0.674 * 92.0).abs() < 1e-3 && (s[1] - 0.337 * 92.0).abs() < 1e-3);
    assert!(m.icons.values().all(|i| i.punch.is_some()));
    assert_eq!(m.state.group_alpha[&1001], 1.0);
    // Opening twice is refused.
    let mut m = m;
    assert!(!m.open(
        &tree(),
        &world(),
        false,
        0.0,
        0.0,
        &mut |_| unreachable!(),
        &fg
    ));
}

#[test]
fn expanding_collapses_siblings_and_places_the_grid_beside_the_parent() {
    let (mut m, mut mk) = opened();
    let w = world();
    let r = m.click("Tools", 0, &w, 0.016, &mut |p| mk.make(p), &fg);
    assert_eq!(r, Some(Clicked::Expanded("Tools".into())));
    assert!(m.placed().iter().any(|(p, _, _)| p == "Tools/I"));
    let r = m.click("Resources", 0, &w, 0.016, &mut |p| mk.make(p), &fg);
    assert_eq!(r, Some(Clicked::Expanded("Resources".into())));
    let placed = m.placed();
    assert!(!placed.iter().any(|(p, _, _)| p == "Tools/I"));
    // Five children: 3 columns × 2 rows right of the 92 wide parent.
    let s = 92.0 / 1.18;
    let a = placed.iter().find(|(p, _, _)| p == "Resources/A").unwrap();
    assert!((a.1[0] - (46.0 + 46.0 + 0.5 * s)).abs() < 1e-3);
    assert!((a.1[1] - (92.0 + 0.5 * s)).abs() < 1e-3);
    // The children are made under the parent's icon.
    let parent = m.icons["Resources"].nodes.root;
    assert_eq!(mk.1.iter().filter(|&&p| p == parent).count(), 5);
    // Their punches wait 0.1 · i ÷ n.
    let e = m.icons["Resources/E"].punch.unwrap();
    assert!((e.delay - 0.08).abs() < 1e-6);
    // Reopening Tools reuses its icon (no new one).
    let before = m.made;
    m.click("Tools", 0, &w, 0.016, &mut |p| mk.make(p), &fg);
    assert_eq!(m.made, before);
}

#[test]
fn chroma_greys_what_cannot_be_crafted_and_categories_without_any() {
    let (mut m, mut mk) = opened();
    let w = world();
    m.click("Resources", 0, &w, 0.016, &mut |p| mk.make(p), &fg);
    m.update(&w, false, 0.0, 0.016);
    assert_eq!(m.icons["Resources/A"].chroma, 1.0);
    assert_eq!(m.icons["Resources/B"].chroma, 0.0);
    // Collapsed children keep their chroma (only an expanded parent's are
    // updated).
    m.click("Tools", 0, &w, 0.016, &mut |p| mk.make(p), &fg);
    m.update(&w, false, 0.0, 0.016);
    let chroma = |p: &str| m.icons[p].chroma;
    assert_eq!(chroma("Resources"), 1.0);
    assert_eq!(chroma("Resources/B"), 0.0);
    // Survival's only recipe lacks ingredients; Tools' is locked.
    assert_eq!(chroma("Survival"), 0.0);
    assert_eq!(chroma("Tools"), 0.0);
    assert_eq!(chroma("Tools/I"), 0.0);
    let bg = m.icons["Resources/B"].nodes.background;
    assert!(m.state.material[&bg].keywords["GRAYSCALE"]);
    assert_eq!(m.state.material[&bg].floats["_Chroma"], 0.0);
}

#[test]
fn a_craft_locks_fades_shows_progress_and_unlocks_when_done() {
    let (mut m, mut mk) = opened();
    let w = world();
    m.click("Resources", 0, &w, 0.016, &mut |p| mk.make(p), &fg);
    // Not affordable: denied, still punched.
    assert_eq!(
        m.click("Resources/B", 0, &w, 0.016, &mut |p| mk.make(p), &fg),
        Some(Clicked::Denied("Resources/B".into()))
    );
    assert_eq!(
        m.click("Resources/A", 0, &w, 0.016, &mut |p| mk.make(p), &fg),
        Some(Clicked::Craft(1))
    );
    assert!(m.locked && m.interactable);
    // Locked: everything grey.
    m.update(&w, true, 0.0, 0.016);
    assert_eq!(m.icons["Resources/A"].chroma, 0.0);
    // `Lock` when the craft begins (≤ 5.9 s): the fade, ease-in-sine.
    m.lock();
    assert!(!m.interactable);
    assert_eq!(m.hit([46.0, 92.0]), None);
    m.update(&w, true, 0.15, 0.016);
    assert!((m.alpha - ease_in_sine(0.5)).abs() < 1e-6);
    assert!((m.icons["Resources/A"].progress - 0.15).abs() < 1e-6);
    assert!((m.icons["Resources"].progress - 0.15).abs() < 1e-6);
    m.update(&w, true, 0.5, 0.016);
    assert_eq!(m.alpha, 0.0);
    // Done: interactable, opaque, progress full, unlocked.
    m.update(&w, false, 0.0, 0.016);
    assert!(m.interactable && !m.locked && m.crafted.is_none());
    assert_eq!(m.alpha, 1.0);
    let fg_node = m.icons["Resources/A"].nodes.foreground;
    assert!(!m.state.material[&fg_node].keywords["FILL_RADIAL"]);
    m.update(&w, false, 0.0, 0.016);
    assert_eq!(m.icons["Resources/A"].chroma, 1.0);
    // Not interactable: clicks are ignored.
    m.lock();
    assert_eq!(
        m.click("Resources/A", 0, &w, 0.016, &mut |p| mk.make(p), &fg),
        None
    );
}

#[test]
fn hover_swaps_the_background_and_the_hit_finds_the_icon() {
    let (mut m, mut mk) = opened();
    let w = world();
    assert_eq!(m.hit([46.0, 92.0]), Some("Resources".into()));
    assert_eq!(m.hit([46.0, 46.0 - 0.5]), Some("Survival".into()));
    assert_eq!(m.hit([200.0, 0.0]), None);
    m.hover(Some("Resources"));
    let bg = m.icons["Resources"].nodes.background;
    assert_eq!(m.state.sprite[&bg], Some(SpriteId(2)));
    m.hover(None);
    assert_eq!(m.state.sprite[&bg], Some(SpriteId(1)));
    m.click("Resources", 0, &w, 0.016, &mut |p| mk.make(p), &fg);
    m.hover(Some("Resources/A"));
    let bg = m.icons["Resources/A"].nodes.background;
    assert_eq!(m.state.sprite[&bg], Some(SpriteId(4)));
    // Right button on a known recipe: the pin (not ported).
    assert_eq!(
        m.click("Resources/A", 1, &w, 0.016, &mut |p| mk.make(p), &fg),
        Some(Clicked::Pin(1))
    );
}

#[test]
fn the_punch_decays_to_rest() {
    let p = Punch {
        tween: CoroutineTween::new(TweenMode::Once, 1.0),
        frequency: 5.0,
        amplitude: 1.0,
        duration: 1.0,
        delay: 0.0,
        seed: 0.0,
    };
    // t = 0: amplitude ½ around 1 (the seed 0: no wave yet).
    assert_eq!(p.scale(0.0), [1.5, 1.5]);
    let end = p.scale(1.0);
    assert!((end[0] - 1.0).abs() < 0.01 && (end[1] - 1.0).abs() < 0.01);
    let (mut m, _) = opened();
    for _ in 0..90 {
        m.update(&world(), false, 0.0, 1.0 / 60.0);
    }
    assert!(
        m.icons
            .values()
            .all(|i| i.punch.is_none() && i.scale == [1.0, 1.0])
    );
}

#[test]
fn close_hides_every_icon_and_resync_adds_new_recipes() {
    let (mut m, mut mk) = opened();
    let w = world();
    m.click("Resources", 0, &w, 0.016, &mut |p| mk.make(p), &fg);
    // Recipe 8 becomes known: the hidden category appears.
    let all = |_: i32| Unlock::Available;
    let w2 = MenuWorld {
        unlock: &all,
        ..world()
    };
    m.resync(&tree(), &w2, &mut |p| mk.make(p), &fg);
    let placed = m.placed();
    assert_eq!(
        placed
            .iter()
            .filter(|(p, _, _)| !p.contains('/'))
            .map(|(p, _, _)| p.as_str())
            .collect::<Vec<_>>(),
        ["Resources", "Survival", "Empty", "Tools"]
    );
    // Four in the column now: Resources moved up.
    assert_eq!(placed[0].1, [46.0, 138.0]);
    assert!(placed.iter().any(|(p, _, _)| p == "Resources/E"));
    let roots: Vec<usize> = m.icons.values().map(|i| i.nodes.root).collect();
    assert!(m.close());
    assert!(m.icons.is_empty() && !m.selected);
    assert!(roots.iter().all(|r| !m.state.active[r]));
}
