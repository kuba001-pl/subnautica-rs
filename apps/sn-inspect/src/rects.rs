//! `ui --rects` (M9u4a, `docs/DESIGN.md` "M9u plan", `docs/formats/ui.md`
//! § Layout): the in-scope UI trees laid out by our port of uGUI's
//! layout system, from their stored RectTransforms. Every value the
//! layout drives is compared with the stored one (the hypothesis: the
//! editor saved what it last computed). Values that need a text's size
//! (M9u4c) or an unported component are counted apart.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, ScriptAssemblies, UiTree};
use sn_install::GameData;
use sn_ui::layout::{RectTransform, layout_with};

use crate::Result;

/// Within this, a value equals the stored one (pixels; anchors are 0–1).
const TOLERANCE: f32 = 0.01;

#[derive(Default)]
struct Tally {
    equal: usize,
    unknown: usize,
    misses: Vec<String>,
    /// Driven values stored as exactly 0, and those that are not.
    stored_zero: usize,
    stored_nonzero: Vec<String>,
    /// Zero-size rects (equal anchors, size delta 0) that nothing drives.
    undriven_zero: Vec<String>,
}

impl Tally {
    fn compare(&mut self, what: String, ours: f32, stored: f32) {
        if stored == 0.0 {
            self.stored_zero += 1;
        } else {
            self.stored_nonzero
                .push(format!("{what}: stored {stored}, ours {ours}"));
        }
        if ours.is_nan() {
            self.unknown += 1;
        } else if (ours - stored).abs() <= TOLERANCE {
            self.equal += 1;
        } else {
            self.misses
                .push(format!("{what}: ours {ours}, stored {stored}"));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_tree(
    assets: &Assets,
    layouts: &sn_assets::Layouts,
    ui: &UiTree,
    tally: &mut Tally,
    unported: &mut BTreeMap<String, usize>,
    stability: &mut usize,
    fonts: &sn_assets::FontAssets,
    set: &sn_ui::text::FontSet,
) -> Result<()> {
    let texts = sn_ui::text::Texts {
        fonts: set,
        params: assets
            .ui_texts(layouts, ui, fonts)?
            .into_iter()
            .map(|t| (t.node, t.params))
            .collect(),
    };
    let built = assets.ui_layout(layouts, ui)?;
    for n in &built.notes {
        println!("  note: {n}");
    }
    let mut tree = built.tree;
    let stored: Vec<RectTransform> = tree.nodes.iter().map(|n| n.rect).collect();
    let report = layout_with(&mut tree, &texts);
    for (k, v) in &report.unported {
        *unported.entry(k.clone()).or_default() += v;
    }
    let (before_eq, before_unknown, before_miss) = (tally.equal, tally.unknown, tally.misses.len());
    let mut driven_nodes = 0;
    for (i, d) in report.driven.iter().enumerate() {
        if !d.any() {
            continue;
        }
        driven_nodes += 1;
        let (ours, want) = (&tree.nodes[i].rect, &stored[i]);
        let path = &ui.nodes[i].path;
        if d.anchors {
            for a in 0..2 {
                tally.compare(
                    format!("{path} anchorMin[{a}]"),
                    ours.anchor_min[a],
                    want.anchor_min[a],
                );
                tally.compare(
                    format!("{path} anchorMax[{a}]"),
                    ours.anchor_max[a],
                    want.anchor_max[a],
                );
            }
        }
        for a in 0..2 {
            if d.position[a] {
                tally.compare(
                    format!("{path} anchoredPosition[{a}]"),
                    ours.anchored_position[a],
                    want.anchored_position[a],
                );
            }
            if d.size[a] {
                tally.compare(
                    format!("{path} sizeDelta[{a}]"),
                    ours.size_delta[a],
                    want.size_delta[a],
                );
            }
        }
    }
    for (i, d) in report.driven.iter().enumerate() {
        let r = &stored[i];
        for a in 0..2 {
            if !d.size[a]
                && tree.active(i)
                && r.anchor_min[a] == r.anchor_max[a]
                && r.size_delta[a] == 0.0
            {
                tally
                    .undriven_zero
                    .push(format!("{} sizeDelta[{a}]", ui.nodes[i].path));
            }
        }
    }
    // Once more from our own result: a settled layout does not move.
    let first: Vec<RectTransform> = tree.nodes.iter().map(|n| n.rect).collect();
    layout_with(&mut tree, &texts);
    let moved = tree
        .nodes
        .iter()
        .zip(&first)
        .filter(|(n, f)| {
            let a = [n.rect.anchored_position, n.rect.size_delta].concat();
            let b = [f.anchored_position, f.size_delta].concat();
            a.iter()
                .zip(&b)
                .any(|(x, y)| !(x.is_nan() && y.is_nan()) && (x - y).abs() > TOLERANCE)
        })
        .count();
    *stability += moved;
    println!(
        "{}: {} nodes, {} layout roots, {driven_nodes} nodes driven; values equal {}, unknown {}, differ {}; {moved} nodes move on a second run",
        ui.key,
        ui.nodes.len(),
        report.roots.len(),
        tally.equal - before_eq,
        tally.unknown - before_unknown,
        tally.misses.len() - before_miss
    );
    Ok(())
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let assemblies = ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let catalog = assets.catalog()?;
    let root = assets.ugui_root(&catalog)?;
    let mut trees = vec![assets.ui_tree("uGUI.prefab", &root)?];
    for o in crate::ui::other_roots(&assets, &root)? {
        let t = assets.ui_tree("uGUI.prefab (other root)", &o)?;
        let name = t.nodes.first().map_or(String::new(), |n| n.name.clone());
        let mut t = t;
        t.key = format!("uGUI.prefab root {name}");
        trees.push(t);
    }
    let main = assets.scene("main")?;
    let (screen, _) = assets.pda_screen_refs(&main)?;
    trees.push(assets.ui_tree("PDA screen", &screen)?);
    let mut tally = Tally::default();
    let mut unported = BTreeMap::new();
    let mut stability = 0;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    for t in &trees {
        check_tree(
            &assets,
            &layouts,
            t,
            &mut tally,
            &mut unported,
            &mut stability,
            &fonts,
            &set,
        )?;
    }
    for m in &tally.stored_nonzero {
        println!("  driven but stored non-zero: {m}");
    }
    for m in &tally.undriven_zero {
        println!("  zero size, not driven by us: {m}");
    }
    println!("unported layout components met (active, enabled): {unported:?}");
    let driven = tally.stored_zero + tally.stored_nonzero.len();
    println!(
        "driven values: {driven}; stored as 0 (zeroed when saved): {}, stored non-zero: {}; ours equal to the stored {} (as a value oracle this holds only where ours is 0), unknown {} (text layout or unported); active zero-size rects not driven: {}; {stability} nodes move on a second run ({:.1} s)",
        tally.stored_zero,
        tally.stored_nonzero.len(),
        tally.equal,
        tally.unknown,
        tally.undriven_zero.len(),
        start.elapsed().as_secs_f64()
    );
    Ok(if tally.stored_nonzero.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
