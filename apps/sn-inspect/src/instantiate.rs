//! `ui --instantiate` (M9u7a, `docs/formats/ui.md` § Objects made at run
//! time): `UiScene::graft` (`Object.Instantiate` + `SetParent(parent,
//! false)`) and `set_as_last_sibling` on the real `uGUI`:
//! 1. a copy of a stored subtree (the depth readout, `PlayerDepth`: images
//!    and texts) grafted beside it, given the stored nodes' start state and
//!    moved by a known offset, draws the same items as the original
//!    (count, kinds, textures, materials, colours, indices) with every
//!    vertex moved by that offset, after the original's;
//! 2. four copies of `ErrorMessage.prefabMessage` (`MessageInstance`, a
//!    root of its own in `uGUI.prefab`'s file) under its `messageCanvas`,
//!    each with its own text, draw in sibling order;
//! 3. `SetAsLastSibling` on the first copy moves its draw after the others;
//! 4. the stored nodes draw exactly as before the grafts.

use std::collections::HashMap;
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, LocalTransform, UiState};
use sn_install::GameData;
use sn_ui::draw::DrawItem;
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};
use sn_unity::Value;

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
/// The copy's move, canvas units.
const OFFSET: [f32; 2] = [37.0, -23.0];

/// The items of `nodes`, in draw order.
fn items_of<'a>(items: &'a [DrawItem], nodes: &[usize]) -> Vec<&'a DrawItem> {
    items.iter().filter(|i| nodes.contains(&i.node)).collect()
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let tags = sn_assets::physics_settings(&assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let catalog = assets.catalog()?;
    let root = assets.ugui_root(&catalog)?;
    let tree = assets.ui_tree("uGUI.prefab", &root)?;
    let layers = &tags.sorting_layers;
    let start = UiState::from(&assets.ui_start_state(&layouts, &tree)?);
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, layers)?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let camera = UiCamera::game(DEFAULT_FIELD_OF_VIEW, NEAR, FAR);
    let mut placements = HashMap::new();
    for (node, s) in assets.ui_scalers(&tree)? {
        if let Ok(p) = place(&s, &camera, DEFAULT_FIELD_OF_VIEW, SCREEN, DEFAULT_UI_SCALE) {
            placements.insert(node, p);
        }
    }
    let (mut ok, mut all) = (0usize, 0usize);
    let mut check = |good: bool, what: &str| {
        all += 1;
        if good {
            ok += 1;
        } else {
            println!("  FAIL: {what}");
        }
    };
    let before = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &start)?;
    let stored = scene.node_count();

    // 1. A stored subtree copied beside itself.
    let original = tree
        .nodes
        .iter()
        .position(|n| n.path.ends_with("/DepthCompass/PlayerDepth"))
        .ok_or("no PlayerDepth")?;
    let parent = tree.nodes[original]
        .parent
        .ok_or("PlayerDepth has no parent")?;
    let go = tree.nodes[original]
        .game_object
        .as_ref()
        .ok_or("PlayerDepth: no GameObject")?;
    let sub = assets.ui_tree("PlayerDepth", go)?;
    let len = sub.nodes.len();
    // The subtree is contiguous in the depth-first scene, in the same order.
    check(
        sub.nodes
            .iter()
            .zip(&tree.nodes[original..])
            .all(|(a, b)| a.name == b.name),
        "the subtree read alone has the scene's node order",
    );
    let template = assets.ui_scene(&layouts, &sub, &fonts, layers)?;
    let copy = scene.graft(&template, parent)?;
    check(copy == stored, "the copy's nodes are appended");
    println!(
        "copy of {} ({len} nodes) grafted as nodes {copy}..{}: {}",
        tree.nodes[original].path,
        copy + len,
        scene.tree.nodes[copy].path
    );
    // The copy gets the original nodes' start state, then the move.
    let map = |n: usize| {
        (original..original + len)
            .contains(&n)
            .then(|| n - original + copy)
    };
    let mut state = start.clone();
    macro_rules! mirror {
        ($f:ident) => {
            let extra: Vec<_> = start
                .$f
                .iter()
                .filter_map(|(k, v)| map(*k).map(|c| (c, v.clone())))
                .collect();
            state.$f.extend(extra);
        };
    }
    mirror!(active);
    mirror!(group_alpha);
    mirror!(renderer_alpha);
    mirror!(enabled);
    mirror!(color);
    mirror!(sprite);
    mirror!(fill_amount);
    mirror!(text);
    mirror!(material);
    mirror!(transform);
    let layout = assets.ui_layout(&layouts, &tree)?;
    let stored_pos = layout.tree.nodes[original].rect.anchored_position;
    let moved = state
        .transform
        .entry(copy)
        .or_insert_with(LocalTransform::default);
    moved.anchored_position = Some([stored_pos[0] + OFFSET[0], stored_pos[1] + OFFSET[1]]);
    let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
    let originals: Vec<usize> = (original..original + len).collect();
    let copies: Vec<usize> = (copy..copy + len).collect();
    let a = items_of(&d.list.items, &originals);
    let b = items_of(&d.list.items, &copies);
    println!("original draws {}, copy draws {}", a.len(), b.len());
    check(
        !a.is_empty() && a.len() == b.len(),
        "the copy draws as many items",
    );
    let mut worst = 0.0f32;
    let mut same = true;
    for (x, y) in a.iter().zip(&b) {
        same &= y.node == x.node - original + copy
            && x.kind == y.kind
            && x.texture == y.texture
            && x.material == y.material
            && x.indices == y.indices
            && x.vertices.len() == y.vertices.len();
        for (u, v) in x.vertices.iter().zip(&y.vertices) {
            same &= u.color == v.color && u.uv == v.uv;
            let e = (v.pos[0] - u.pos[0] - OFFSET[0])
                .abs()
                .max((v.pos[1] - u.pos[1] - OFFSET[1]).abs());
            worst = worst.max(e);
        }
    }
    println!("copy vs original: largest vertex error after the move {worst:.6} canvas units");
    check(
        same,
        "each copy item matches its original (node, kind, texture, material, indices, colours, UVs)",
    );
    check(
        worst < 1e-3,
        "every copy vertex is the original's moved by the offset",
    );
    let pos = |n: &[&DrawItem]| {
        n.first()
            .and_then(|f| d.list.items.iter().position(|i| std::ptr::eq(i, *f)))
    };
    check(
        pos(&b) > pos(&a),
        "the copy draws after the original (its parent's last child)",
    );

    // 2. Four `MessageInstance` copies.
    let (_, em) = tree
        .scripts("ErrorMessage")
        .next()
        .ok_or("no ErrorMessage")?;
    let (_, f) = assets.script_fields(&layouts, em)?;
    let ptr = |n: &str| {
        f.get(n)
            .and_then(Value::as_pptr)
            .filter(|p| !p.is_null())
            .ok_or_else(|| format!("ErrorMessage.{n}: not set"))
    };
    let prefab = assets
        .resolve(&em.object.file, ptr("prefabMessage")?)?
        .ok_or("prefabMessage not found")?;
    // `messageCanvas` is a RectTransform: the node holding it.
    let rect = assets
        .resolve(&em.object.file, ptr("messageCanvas")?)?
        .ok_or("messageCanvas not found")?;
    let canvas = tree
        .nodes
        .iter()
        .position(|n| n.components.iter().any(|c| c.object.key() == rect.key()))
        .ok_or("messageCanvas not in the tree")?;
    let message_tree = assets.ui_tree("MessageInstance", &prefab)?;
    println!(
        "prefabMessage: {} ({} nodes); messageCanvas {}",
        message_tree.nodes[0].name,
        message_tree.nodes.len(),
        tree.nodes[canvas].path
    );
    let message = assets.ui_scene(&layouts, &message_tree, &fonts, layers)?;
    let mut entries = Vec::new();
    for _ in 0..4 {
        entries.push(scene.graft(&message, canvas)?);
    }
    let mut state = start.clone();
    for (k, &e) in entries.iter().enumerate() {
        state.active.insert(e, true);
        state.text.insert(e, format!("Message {k}"));
        state.transform.insert(
            e,
            LocalTransform {
                anchored_position: Some([0.0, -40.0 * k as f32]),
                ..LocalTransform::default()
            },
        );
    }
    let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
    let order: Vec<usize> = d
        .list
        .items
        .iter()
        .filter(|i| entries.contains(&i.node))
        .map(|i| i.node)
        .collect();
    println!("messages: draws by node {order:?}");
    let mut firsts = order.clone();
    firsts.dedup();
    check(firsts == entries, "the four messages draw in sibling order");

    // 3. `SetAsLastSibling` on the first.
    check(scene.set_as_last_sibling(entries[0]), "SetAsLastSibling");
    let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
    let mut order: Vec<usize> = d
        .list
        .items
        .iter()
        .filter(|i| entries.contains(&i.node))
        .map(|i| i.node)
        .collect();
    order.dedup();
    println!("after SetAsLastSibling({}): {order:?}", entries[0]);
    check(
        order == vec![entries[1], entries[2], entries[3], entries[0]],
        "the first message now draws last",
    );

    // 4. The stored nodes draw as before.
    let after = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &start)?;
    let old: Vec<&DrawItem> = after
        .list
        .items
        .iter()
        .filter(|i| i.node < stored)
        .collect();
    check(
        old.len() == before.list.items.len()
            && old.iter().zip(&before.list.items).all(|(x, y)| **x == *y),
        "the stored nodes draw exactly as before the grafts",
    );
    println!(
        "{} stored items before, {} after (+{} from the copies); checks {ok} of {all}",
        before.list.items.len(),
        old.len(),
        after.list.items.len() - old.len()
    );
    Ok(if ok == all {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
