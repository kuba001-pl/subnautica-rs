//! `ui` (M9u0, `docs/DESIGN.md` "M9u plan"): a census of the game's UI,
//! headless. Where `uGUI.prefab` and the PDA's screen come from and what
//! they hold, every canvas and canvas scaler, the hosts of the scripts
//! M9u ports, the sprite atlases and the fonts; each count checked
//! against the one the plan was written with.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, FileRef, ObjectRef, UiTree};
use sn_install::GameData;
use sn_unity::{
    Canvas, CanvasScaler, GameObject, MonoBehaviourHeader, RENDER_MODE_SCREEN_SPACE_CAMERA,
    RENDER_MODE_SCREEN_SPACE_OVERLAY, RENDER_MODE_WORLD_SPACE, SpriteAtlas, Texture2D,
    TransformNode, class_name, object_name,
};

use crate::Result;

const MONO_BEHAVIOUR: i32 = 114;
const TEXTURE_2D: i32 = 28;
const FONT: i32 = 128;
const SPRITE: i32 = 213;
const CANVAS: i32 = 223;
const RECT_TRANSFORM: i32 = 224;
const CANVAS_GROUP: i32 = 225;
const CANVAS_RENDERER: i32 = 222;
const SPRITE_ATLAS: i32 = 687_078_895;

/// The scripts M9u ports (the plan's in-scope screens), listed with the
/// objects that hold them.
const IN_SCOPE: &[&str] = &[
    "uGUI",
    "uGUI_SceneHUD",
    "uGUI_CanvasScaler",
    "uGUI_OxygenBar",
    "uGUI_HealthBar",
    "uGUI_FoodBar",
    "uGUI_WaterBar",
    "uGUI_CircularBar",
    "uGUI_DepthCompass",
    "HandReticle",
    "uGUI_HandReticleIcon",
    "ErrorMessage",
    "uGUI_IconNotifier",
    "uGUI_PopupNotification",
    "uGUI_PopupNotificationDefault",
    "uGUI_NotificationLabel",
    "uGUI_QuickSlots",
    "uGUI_PlayerDeath",
    "uGUI_SceneRespawning",
    "uGUI_Fader",
    "uGUI_Overlays",
    "uGUI_Tooltip",
    "uGUI_PDA",
    "uGUI_InventoryTab",
    "uGUI_ItemsContainer",
    "uGUI_Equipment",
    "uGUI_ItemIcon",
];

/// Checks against the numbers the plan was written with.
struct Checks {
    failed: usize,
}

impl Checks {
    fn eq(&mut self, what: &str, got: usize, want: usize) {
        let ok = got == want;
        if !ok {
            self.failed += 1;
        }
        println!(
            "  check {what}: {got} (plan: {want}) {}",
            if ok { "ok" } else { "DIFFERS" }
        );
    }
}

fn mode_name(mode: i32) -> &'static str {
    match mode {
        RENDER_MODE_SCREEN_SPACE_OVERLAY => "screen-space overlay",
        RENDER_MODE_SCREEN_SPACE_CAMERA => "screen-space camera",
        RENDER_MODE_WORLD_SPACE => "world space",
        _ => "?",
    }
}

/// `uGUI_CanvasScaler.Mode` and `.ScaleMode` (`code --enum`).
fn scaler_mode(mode: i32) -> &'static str {
    match mode {
        0 => "Static",
        1 => "Inversed",
        2 => "Parented",
        3 => "World",
        _ => "?",
    }
}

fn scale_mode(mode: i32) -> &'static str {
    match mode {
        0 => "DontScale",
        1 => "Immediate",
        2 => "Postponed",
        _ => "?",
    }
}

fn data(object: &ObjectRef) -> Result<(&[u8], bool)> {
    let (_, d) = object.data()?;
    Ok((d, object.file.file().big_endian))
}

fn print_tree(tree: &UiTree) {
    let active = tree.nodes.iter().filter(|n| n.active).count();
    let mut layers: BTreeMap<u32, usize> = BTreeMap::new();
    for n in &tree.nodes {
        *layers.entry(n.layer).or_default() += 1;
    }
    println!(
        "  nodes {} ({active} active in the hierarchy), by layer {layers:?}",
        tree.nodes.len()
    );
    println!("  components by class:");
    for (class, n) in tree.class_counts() {
        println!("    {n:5} {class:>4} {}", class_name(class).unwrap_or("?"));
    }
    let mut scripts: Vec<(String, usize)> = tree.script_counts().into_iter().collect();
    scripts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("  MonoBehaviours by script ({} scripts):", scripts.len());
    for (s, n) in &scripts {
        println!("    {n:5} {s}");
    }
}

/// Canvases and canvas scalers of a tree; returns (canvases, world-space
/// canvases, overlay canvases, scalers).
fn print_canvases(tree: &UiTree) -> Result<(usize, usize, usize, usize)> {
    let (mut canvases, mut world, mut overlay, mut scalers) = (0, 0, 0, 0);
    for (i, node) in tree.nodes.iter().enumerate() {
        for c in &node.components {
            if c.class_id == CANVAS {
                let (d, be) = data(&c.object)?;
                let cv = Canvas::parse(d, be).map_err(|e| format!("{}: Canvas: {e}", node.path))?;
                canvases += 1;
                match cv.render_mode {
                    RENDER_MODE_WORLD_SPACE => world += 1,
                    RENDER_MODE_SCREEN_SPACE_OVERLAY => overlay += 1,
                    _ => {}
                }
                println!(
                    "  canvas {:?}{}: {}, enabled {}, plane distance {}, sorting layer {} order {}{}, pixel perfect {}, shader channels {:#x}",
                    node.path,
                    if node.active { "" } else { " [inactive]" },
                    mode_name(cv.render_mode),
                    cv.enabled,
                    cv.plane_distance,
                    cv.sorting_layer_id,
                    cv.sorting_order,
                    if cv.override_sorting {
                        " (override)"
                    } else {
                        ""
                    },
                    cv.pixel_perfect,
                    cv.additional_shader_channels,
                );
            }
            if c.script.as_deref() == Some("uGUI_CanvasScaler") {
                let (d, be) = data(&c.object)?;
                let s = CanvasScaler::parse(d, be)
                    .map_err(|e| format!("{}: uGUI_CanvasScaler: {e}", node.path))?;
                scalers += 1;
                println!(
                    "  scaler {:?} (node {i}): enabled {}, mode {}, vr mode {}, distance {} m, reference {}×{}, scale {}",
                    node.path,
                    s.enabled,
                    scaler_mode(s.mode),
                    scaler_mode(s.vr_mode),
                    s.distance,
                    s.reference_resolution[0],
                    s.reference_resolution[1],
                    scale_mode(s.scale_mode),
                );
            }
        }
    }
    Ok((canvases, world, overlay, scalers))
}

fn print_hosts(tree: &UiTree) {
    for class in IN_SCOPE {
        for (i, _) in tree.scripts(class) {
            let n = &tree.nodes[i];
            println!(
                "  {class:<30} {:?}{}",
                n.path,
                if n.active { "" } else { " [inactive]" }
            );
        }
    }
}

/// The atlases bundle: every `SpriteAtlas`, and where its packed sprites
/// resolve to. Returns (atlases, the `Items` atlas's packed sprites, of
/// those resolved to a Sprite).
fn atlases(assets: &Assets) -> Result<(usize, usize, usize)> {
    let bundle = assets
        .bundle_named("atlases_")
        .ok_or("no atlases_ bundle")?
        .to_path_buf();
    println!(
        "atlases: {}",
        bundle.file_name().unwrap_or_default().to_string_lossy()
    );
    let loaded = assets.bundle(&bundle)?;
    let names: Vec<String> = loaded
        .file_names()
        .filter(|n| !n.ends_with(".resS"))
        .map(String::from)
        .collect();
    let (mut count, mut items, mut items_resolved) = (0, 0, 0);
    for name in &names {
        let file = assets.file(&bundle, name)?;
        for e in &file.file().externals {
            println!("  external {}", e.path);
        }
        let mut pages = Vec::new();
        for info in file.objects() {
            let Some(d) = file.file().object_data(file.bytes(), info) else {
                continue;
            };
            let be = file.file().big_endian;
            match info.class_id {
                SPRITE_ATLAS => {
                    let a = SpriteAtlas::parse(d, be).map_err(|e| format!("SpriteAtlas: {e}"))?;
                    let (resolved, by_file) = resolve_sprites(assets, &file, &a)?;
                    count += 1;
                    if a.name == "Items" {
                        items = a.packed_sprites.len();
                        items_resolved = resolved;
                    }
                    println!(
                        "  atlas {:?}: {} packed sprites, {resolved} resolve to a Sprite, in files {by_file:?}",
                        a.name,
                        a.packed_sprites.len()
                    );
                }
                TEXTURE_2D => {
                    let t = Texture2D::parse(d, be).map_err(|e| format!("Texture2D: {e}"))?;
                    pages.push(format!(
                        "{:?} {}×{} format {}",
                        t.name, t.width, t.height, t.format
                    ));
                }
                _ => {}
            }
        }
        for p in pages {
            println!("  page {p}");
        }
    }
    Ok((count, items, items_resolved))
}

fn resolve_sprites(
    assets: &Assets,
    file: &FileRef,
    atlas: &SpriteAtlas,
) -> Result<(usize, BTreeMap<String, usize>)> {
    let mut resolved = 0;
    let mut by_file = BTreeMap::new();
    for p in &atlas.packed_sprites {
        if let Some(o) = assets.resolve(file, *p)?
            && o.data()?.0.class_id == SPRITE
        {
            resolved += 1;
            *by_file.entry(o.file.name.clone()).or_default() += 1;
        }
    }
    Ok((resolved, by_file))
}

/// The fonts bundle. Returns (fonts, TMP font assets, font atlases stored
/// with pixels).
fn fonts(assets: &Assets) -> Result<(usize, usize, usize)> {
    let bundle = assets
        .bundle_named("fonts_assets_all")
        .ok_or("no fonts_assets_all bundle")?
        .to_path_buf();
    println!(
        "fonts: {}",
        bundle.file_name().unwrap_or_default().to_string_lossy()
    );
    let loaded = assets.bundle(&bundle)?;
    let names: Vec<String> = loaded
        .file_names()
        .filter(|n| !n.ends_with(".resS"))
        .map(String::from)
        .collect();
    let (mut fonts, mut font_assets, mut stored) = (0, 0, 0);
    for name in &names {
        let file = assets.file(&bundle, name)?;
        let be = file.file().big_endian;
        for info in file.objects() {
            let Some(d) = file.file().object_data(file.bytes(), info) else {
                continue;
            };
            match info.class_id {
                FONT => {
                    fonts += 1;
                    let n = object_name(d, be).map_err(|e| format!("Font: {e}"))?;
                    println!("  font {n:?}: {} bytes", d.len());
                }
                MONO_BEHAVIOUR => {
                    let h = MonoBehaviourHeader::parse(d, be).map_err(|e| e.to_string())?;
                    let o = ObjectRef {
                        file: file.clone(),
                        path_id: info.path_id,
                    };
                    let script = assets.script_class(&o).unwrap_or_else(|| "?".into());
                    if script == "TMP_FontAsset" {
                        font_assets += 1;
                    }
                    println!("  {script} {:?}: {} bytes", h.name, d.len());
                }
                TEXTURE_2D => {
                    let t = Texture2D::parse(d, be).map_err(|e| format!("Texture2D: {e}"))?;
                    if t.name.ends_with(" Atlas") && t.width > 0 {
                        stored += 1;
                    }
                    if t.width > 0 {
                        println!(
                            "  texture {:?}: {}×{} format {}",
                            t.name, t.width, t.height, t.format
                        );
                    }
                }
                _ => {}
            }
        }
    }
    Ok((fonts, font_assets, stored))
}

/// GameObjects in `root`'s file whose transform has no parent, other
/// than `root`.
fn other_roots(assets: &Assets, root: &ObjectRef) -> Result<Vec<ObjectRef>> {
    let file = &root.file;
    let be = file.file().big_endian;
    let mut out = Vec::new();
    for info in file.objects().iter().filter(|o| o.class_id == 1) {
        if info.path_id == root.path_id {
            continue;
        }
        let Some(d) = file.file().object_data(file.bytes(), info) else {
            continue;
        };
        let go = GameObject::parse(d, be).map_err(|e| e.to_string())?;
        for c in &go.components {
            let Some(t) = assets.resolve(file, *c)? else {
                continue;
            };
            let (ti, td) = t.data()?;
            if ti.class_id == 4 || ti.class_id == RECT_TRANSFORM {
                let tn = TransformNode::parse(td, be).map_err(|e| e.to_string())?;
                if tn.father.is_null() {
                    out.push(ObjectRef {
                        file: file.clone(),
                        path_id: info.path_id,
                    });
                }
                break;
            }
        }
    }
    Ok(out)
}

fn node_name(assets: &Assets, transform: &ObjectRef) -> Result<String> {
    let (d, be) = data(transform)?;
    let t = TransformNode::parse(d, be).map_err(|e| e.to_string())?;
    let Some(go) = assets.resolve(&transform.file, t.game_object)? else {
        return Ok("?".into());
    };
    let (d, be) = data(&go)?;
    Ok(GameObject::parse(d, be).map_err(|e| e.to_string())?.name)
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    let mut checks = Checks { failed: 0 };

    let root = assets.ugui_root(&catalog)?;
    println!(
        "{} (uGUI.InitializeAsync's key) → {} in {}",
        sn_assets::UGUI_PREFAB_KEY,
        root.path_id,
        root.file.name
    );
    let ugui = assets.ui_tree("uGUI.prefab", &root)?;
    print_tree(&ugui);
    let (canvases, world, overlay, scalers) = print_canvases(&ugui)?;
    let counts = ugui.class_counts();
    let scripts = ugui.script_counts();
    let script = |s: &str| scripts.get(s).copied().unwrap_or(0);
    checks.eq("uGUI.prefab nodes", ugui.nodes.len(), 361);
    checks.eq(
        "uGUI.prefab RectTransforms",
        counts.get(&RECT_TRANSFORM).copied().unwrap_or(0),
        360,
    );
    checks.eq("uGUI.prefab canvases", canvases, 12);
    checks.eq("world-space canvases", world, 11);
    checks.eq("screen-space overlay canvases", overlay, 1);
    checks.eq("uGUI_CanvasScaler", scalers, 4);
    checks.eq("Image", script("Image"), 143);
    checks.eq("TextMeshProUGUI", script("TextMeshProUGUI"), 101);
    checks.eq("uGUI_HandReticleIcon", script("uGUI_HandReticleIcon"), 10);
    println!("in-scope scripts in uGUI.prefab:");
    print_hosts(&ugui);

    // The bundle's other prefabs (root objects besides uGUI): the ones its
    // scripts instantiate.
    let others = other_roots(&assets, &root)?;
    println!(
        "other root objects in uGUI.prefab's file ({}):",
        others.len()
    );
    let (mut other_nodes, mut other_rects, mut other_canvases) = (0, 0, 0);
    let mut other_scripts: BTreeMap<String, usize> = BTreeMap::new();
    for o in &others {
        let t = assets.ui_tree("other", o)?;
        let c = t.class_counts();
        other_nodes += t.nodes.len();
        other_rects += c.get(&RECT_TRANSFORM).copied().unwrap_or(0);
        other_canvases += c.get(&CANVAS).copied().unwrap_or(0);
        let mut s: Vec<String> = t
            .script_counts()
            .into_iter()
            .map(|(k, v)| {
                *other_scripts.entry(k.clone()).or_default() += v;
                if v > 1 { format!("{k}×{v}") } else { k }
            })
            .collect();
        s.sort();
        println!(
            "  {:?}: {} nodes; {}",
            t.nodes[0].name,
            t.nodes.len(),
            s.join(", ")
        );
    }
    let both = |s: &str| script(s) + other_scripts.get(s).copied().unwrap_or(0);
    checks.eq(
        "nodes, uGUI and the other roots",
        ugui.nodes.len() + other_nodes,
        387,
    );
    checks.eq(
        "RectTransforms, uGUI and the other roots",
        counts.get(&RECT_TRANSFORM).copied().unwrap_or(0) + other_rects,
        385,
    );
    checks.eq(
        "canvases, uGUI and the other roots",
        canvases + other_canvases,
        13,
    );
    checks.eq("Image, uGUI and the other roots", both("Image"), 148);
    checks.eq(
        "TextMeshProUGUI, uGUI and the other roots",
        both("TextMeshProUGUI"),
        110,
    );

    let main = assets.scene("main")?;
    let (screen, anchor) = assets.pda_screen_refs(&main)?;
    let anchor_name = match &anchor {
        Some(a) => node_name(&assets, a)?,
        None => "null".into(),
    };
    println!(
        "PDA.prefabScreen → {} in {}; PDA.screenAnchor → {anchor_name:?}",
        screen.path_id, screen.file.name
    );
    let pda = assets.ui_tree("PDA screen", &screen)?;
    println!(
        "PDA screen {:?}:",
        pda.nodes.first().map(|n| n.name.as_str()).unwrap_or("")
    );
    print_tree(&pda);
    let (pda_canvases, ..) = print_canvases(&pda)?;
    println!("  ({pda_canvases} canvases)");
    let pda_scripts = pda.script_counts();
    let pda_script = |s: &str| pda_scripts.get(s).copied().unwrap_or(0);
    checks.eq("uGUI_PDA", pda_script("uGUI_PDA"), 1);
    checks.eq("uGUI_InventoryTab", pda_script("uGUI_InventoryTab"), 1);
    println!("in-scope scripts in the PDA screen:");
    print_hosts(&pda);

    let (atlas_count, items, items_resolved) = atlases(&assets)?;
    checks.eq("sprite atlases", atlas_count, 8);
    checks.eq("Items atlas packed sprites", items, 470);
    checks.eq("Items sprites that resolve", items_resolved, 470);

    let (font_count, font_assets, stored) = fonts(&assets)?;
    checks.eq("fonts", font_count, 9);
    checks.eq("TMP font assets", font_assets, 8);
    checks.eq("font atlases stored with pixels", stored, 1);

    println!(
        "{} checks differ ({:.2} s)",
        checks.failed,
        start.elapsed().as_secs_f64()
    );
    Ok(if checks.failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// The serialized files beside the bundles, by name.
const STANDALONE: &[&str] = &[
    "globalgamemanagers",
    "globalgamemanagers.assets",
    "resources.assets",
    "sharedassets0.assets",
    "level0",
];

/// Per script class: objects read to their last byte, objects that were
/// not, and the first error.
#[derive(Default)]
struct ScriptTally {
    ok: usize,
    failed: usize,
    first_error: Option<String>,
}

/// `ui --scripts [--all]` (M9u1): every MonoBehaviour of the UI's files
/// and the startup scenes (`--all`: of every bundle) read with the
/// layout built from the game's assemblies; each must end with its last
/// field.
pub fn scripts(game: &GameData, all: bool) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    println!(
        "assemblies loaded: {} ({})",
        assemblies.files.len(),
        assemblies
            .files
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let layouts = assemblies.layouts()?;
    let bundles: Vec<std::path::PathBuf> = if all {
        assets.bundle_paths().to_vec()
    } else {
        [
            "allprefabs_asset_ugui.prefab",
            "main.unity_",
            "essentials.unity_",
            "fonts_assets_all",
            "atlases_",
        ]
        .iter()
        .filter_map(|p| assets.bundle_named(p).map(|b| b.to_path_buf()))
        .collect()
    };
    let mut tally: BTreeMap<String, ScriptTally> = BTreeMap::new();
    let mut no_script = 0;
    let mut check = |file: &FileRef| {
        let be = file.file().big_endian;
        for info in file
            .objects()
            .iter()
            .filter(|o| o.class_id == MONO_BEHAVIOUR)
        {
            let object = ObjectRef {
                file: file.clone(),
                path_id: info.path_id,
            };
            let Some(script) = assets.mono_script(&object) else {
                no_script += 1;
                continue;
            };
            let class = if script.namespace.is_empty() {
                script.class_name.clone()
            } else {
                format!("{}.{}", script.namespace, script.class_name)
            };
            let t = tally.entry(class).or_default();
            let result = layouts
                .script(&script.assembly, &script.namespace, &script.class_name)
                .and_then(|layout| {
                    let (_, d) = object.data()?;
                    sn_unity::read_script(d, be, &layout).map_err(|e| e.to_string())
                });
            match result {
                Ok(_) => t.ok += 1,
                Err(e) => {
                    t.failed += 1;
                    t.first_error.get_or_insert(e);
                }
            }
        }
    };
    // The player's own files (`Subnautica_Data/*.assets`, `level0`, …).
    let mut standalone_files = 0;
    if all {
        for name in STANDALONE {
            if game.data_dir.join(name).is_file() {
                check(&assets.standalone(name)?);
                standalone_files += 1;
            }
        }
    }
    for (i, bundle) in bundles.iter().enumerate() {
        if i % 256 == 255 {
            assets.trim_cache(0);
        }
        let loaded = assets.bundle(bundle)?;
        let names: Vec<String> = loaded
            .file_names()
            .filter(|n| !n.ends_with(".resS") && !n.ends_with(".resource"))
            .map(String::from)
            .collect();
        for name in names {
            check(&assets.file(bundle, &name)?);
        }
    }
    let ok: usize = tally.values().map(|t| t.ok).sum();
    let failed: usize = tally.values().map(|t| t.failed).sum();
    let failed_classes = tally.values().filter(|t| t.failed > 0).count();
    println!("scripts that do not read to their last byte:");
    for (class, t) in tally.iter().filter(|(_, t)| t.failed > 0) {
        println!(
            "  {class}: {} of {} — {}",
            t.failed,
            t.ok + t.failed,
            t.first_error.as_deref().unwrap_or("")
        );
    }
    println!("fields not stored, by reason (top 25):");
    let skipped = layouts.skipped.borrow();
    let mut reasons: Vec<(&String, &usize)> = skipped.iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (why, n) in reasons.iter().take(25) {
        println!("  {n:6} {why}");
    }
    println!(
        "{standalone_files} standalone files, {} bundles; MonoBehaviours read to their last byte: {ok} of {} ({:.2}%), {failed} not, in {failed_classes} of {} script classes; {no_script} without a script ({:.1} s)",
        bundles.len(),
        ok + failed,
        100.0 * ok as f64 / (ok + failed).max(1) as f64,
        tally.len(),
        start.elapsed().as_secs_f64()
    );
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// `ui --layout <CLASS> [<NAME>]` (M9u1): the layout of script `CLASS`
/// (as `Namespace.Name`) and, for its first object in the UI's files and
/// the startup scenes (named `NAME` if given), where each field starts
/// and its value.
pub fn layout(game: &GameData, class: &str, object_name: Option<&str>) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    for prefix in [
        "allprefabs_asset_ugui.prefab",
        "main.unity_",
        "essentials.unity_",
        "fonts_assets_all",
    ] {
        let Some(bundle) = assets.bundle_named(prefix).map(|b| b.to_path_buf()) else {
            continue;
        };
        let loaded = assets.bundle(&bundle)?;
        let names: Vec<String> = loaded
            .file_names()
            .filter(|n| !n.ends_with(".resS"))
            .map(String::from)
            .collect();
        for name in names {
            let file = assets.file(&bundle, &name)?;
            for info in file
                .objects()
                .iter()
                .filter(|o| o.class_id == MONO_BEHAVIOUR)
            {
                let object = ObjectRef {
                    file: file.clone(),
                    path_id: info.path_id,
                };
                let Some(script) = assets.mono_script(&object) else {
                    continue;
                };
                let full = if script.namespace.is_empty() {
                    script.class_name.clone()
                } else {
                    format!("{}.{}", script.namespace, script.class_name)
                };
                if full != class {
                    continue;
                }
                if let Some(want) = object_name {
                    let (_, d) = object.data()?;
                    let h = MonoBehaviourHeader::parse(d, file.file().big_endian)
                        .map_err(|e| e.to_string())?;
                    if h.name != want {
                        continue;
                    }
                }
                let l = layouts.script(&script.assembly, &script.namespace, &script.class_name)?;
                let (_, d) = object.data()?;
                println!(
                    "{class} in {name}, object {} ({} bytes):",
                    info.path_id,
                    d.len()
                );
                let trace = sn_unity::trace_script(d, file.file().big_endian, &l)
                    .map_err(|e| e.to_string())?;
                for ((field, at, value), f) in trace.iter().zip(&l.fields) {
                    let shown = match value {
                        Ok(v) => {
                            let s = format!("{v:?}");
                            match s.char_indices().nth(100) {
                                Some((i, _)) => format!("{}…", &s[..i]),
                                None => s,
                            }
                        }
                        Err(e) => format!("ERROR {e}"),
                    };
                    println!("  @{at:<6} {field:<32} {:<12} {shown}", kind_name(&f.kind));
                }
                for f in l.fields.iter().skip(trace.len()) {
                    println!("  (not read) {} {}", f.name, kind_name(&f.kind));
                }
                return Ok(ExitCode::SUCCESS);
            }
        }
    }
    Err(format!("no object of script {class}"))
}

fn kind_name(k: &sn_unity::Kind) -> String {
    match k {
        sn_unity::Kind::Array(e) => format!("[{}]", kind_name(e)),
        sn_unity::Kind::Struct(s) => s.type_name.clone(),
        other => format!("{other:?}"),
    }
}

/// `ui --sprites [--icons]` (M9u2): every `Sprite` and `SpriteAtlas` of
/// the game parsed to its last byte; the atlases `SpriteManager` loads and
/// how their sprites are packed; every tech type's icon by the game's
/// rule; 20 `Items` sprites printed for the UnityPy oracle; with
/// `--icons`, the items' icons as PNGs in `out/ui/icons/`.
pub fn sprites(game: &GameData, write_icons: bool) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    let mut checks = Checks { failed: 0 };

    // Every object of the UI's built-in classes in the game.
    const CLASSES: [(i32, &str); 6] = [
        (SPRITE, "Sprite"),
        (SPRITE_ATLAS, "SpriteAtlas"),
        (RECT_TRANSFORM, "RectTransform"),
        (CANVAS, "Canvas"),
        (CANVAS_GROUP, "CanvasGroup"),
        (CANVAS_RENDERER, "CanvasRenderer"),
    ];
    let mut parsed = [0usize; CLASSES.len()];
    let mut failed = [0usize; CLASSES.len()];
    let mut first_errors: Vec<String> = Vec::new();
    let mut files: Vec<FileRef> = Vec::new();
    for name in STANDALONE {
        if game.data_dir.join(name).is_file() {
            files.push(assets.standalone(name)?);
        }
    }
    let mut tally = |file: &FileRef| {
        let be = file.file().big_endian;
        for info in file.objects() {
            let Some(k) = CLASSES.iter().position(|c| c.0 == info.class_id) else {
                continue;
            };
            let Some(d) = file.file().object_data(file.bytes(), info) else {
                continue;
            };
            let ok = match info.class_id {
                SPRITE => sn_unity::Sprite::parse(d, be).map(|_| ()),
                SPRITE_ATLAS => sn_unity::SpriteAtlas::parse(d, be).map(|_| ()),
                RECT_TRANSFORM => sn_unity::RectTransform::parse(d, be).map(|_| ()),
                CANVAS => Canvas::parse(d, be).map(|_| ()),
                CANVAS_GROUP => sn_unity::CanvasGroup::parse(d, be).map(|_| ()),
                _ => sn_unity::CanvasRenderer::parse(d, be).map(|_| ()),
            };
            match ok {
                Ok(()) => parsed[k] += 1,
                Err(e) => {
                    failed[k] += 1;
                    if first_errors.len() < 10 {
                        first_errors.push(format!(
                            "{} {} {}: {e}",
                            CLASSES[k].1, file.name, info.path_id
                        ));
                    }
                }
            }
        }
    };
    for f in &files {
        tally(f);
    }
    for (i, bundle) in assets.bundle_paths().to_vec().iter().enumerate() {
        if i % 256 == 255 {
            assets.trim_cache(0);
        }
        let loaded = assets.bundle(bundle)?;
        let names: Vec<String> = loaded
            .file_names()
            .filter(|n| !n.ends_with(".resS") && !n.ends_with(".resource"))
            .map(String::from)
            .collect();
        for name in names {
            tally(&assets.file(bundle, &name)?);
        }
    }
    for e in &first_errors {
        println!("  error: {e}");
    }
    for (k, (_, name)) in CLASSES.iter().enumerate() {
        println!(
            "{name}: {} parsed to their last byte, {} not",
            parsed[k], failed[k]
        );
    }
    checks.eq("sprites parsed", parsed[0], 1190);
    checks.eq("atlases parsed", parsed[1], 8);
    checks.eq("UI objects not parsed", failed.iter().sum(), 0);

    // The atlases SpriteManager loads.
    let sprites = assets.sprites(&catalog)?;
    let mut names: Vec<&String> = sprites.atlases.keys().collect();
    names.sort();
    for name in names {
        let a = &sprites.atlases[name];
        let mut settings: BTreeMap<String, usize> = BTreeMap::new();
        let mut pages: BTreeMap<i64, usize> = BTreeMap::new();
        let mut sliced = 0;
        for s in a.sprites.values() {
            let p = &s.packed;
            *settings
                .entry(format!(
                    "packed {} mode {} rotation {} mesh {}",
                    u8::from(p.packed()),
                    if p.packing_mode() == 0 {
                        "tight"
                    } else {
                        "rect"
                    },
                    p.packing_rotation(),
                    (p.settings_raw >> 6) & 1
                ))
                .or_default() += 1;
            *pages.entry(s.page.path_id).or_default() += 1;
            if s.sprite.border.iter().any(|&b| b > 0.0) {
                sliced += 1;
            }
        }
        let mut page_files: Vec<String> = a
            .sprites
            .values()
            .map(|s| {
                s.page
                    .file
                    .bundle
                    .path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            })
            .collect();
        page_files.sort();
        page_files.dedup();
        println!("  atlas {name:?}: pages in {page_files:?}");
        println!(
            "  atlas {name:?} (label {}): {} sprites, {} pages, {sliced} with borders (nine-slice); {settings:?}",
            sn_assets::SPRITE_ATLASES_LABEL,
            a.sprites.len(),
            pages.len()
        );
    }
    for (name, why) in &sprites.misses {
        println!("  miss {name}: {why}");
    }
    checks.eq("atlases loaded by the label", sprites.atlases.len(), 8);
    checks.eq(
        "Items sprites resolved",
        sprites.atlases.get("Items").map_or(0, |a| a.sprites.len()),
        470,
    );
    checks.eq("sprites that did not resolve", sprites.misses.len(), 0);
    let fallback = sprites.default_sprite();
    println!(
        "  defaultSprite (Default/Unknown): {}",
        fallback.map_or("MISSING".to_string(), |s| format!(
            "{:?} {:?}",
            s.sprite.name, s.packed.texture_rect
        ))
    );
    checks.eq("defaultSprite found", usize::from(fallback.is_some()), 1);

    // Every tech type's icon by the game's rule.
    let dll = sn_assets::read_assembly(game)?;
    let code = sn_assets::game_code(&dll)?;
    let own = code
        .tech_types
        .iter()
        .filter(|(_, n)| sprites.get_no_default(sn_assets::GROUP_ITEM, n).is_some())
        .count();
    println!(
        "  tech types: {} names, {own} with their own icon, {} get defaultSprite",
        code.tech_types.len(),
        code.tech_types.len() - own
    );
    let (rules, _) = sn_assets::item_rules(game, &assets, &dll)?;
    let items: Vec<(i32, String)> = rules
        .tech_class
        .keys()
        .map(|&t| (t, code.tech_name(t).unwrap_or("?").to_string()))
        .collect();
    let missing: Vec<&str> = items
        .iter()
        .filter(|(_, n)| sprites.get_no_default(sn_assets::GROUP_ITEM, n).is_none())
        .map(|(_, n)| n.as_str())
        .collect();
    println!(
        "  item tech types (ItemRules): {}, {} without their own icon {missing:?}",
        items.len(),
        missing.len()
    );

    // For the oracle (out/m9u/oracle/sprites.py prints the same).
    let mut oracle: Vec<&sn_assets::AtlasSprite> = sprites
        .atlases
        .get("Items")
        .map(|a| a.sprites.values().collect())
        .unwrap_or_default();
    oracle.sort_by(|a, b| a.sprite.name.cmp(&b.sprite.name));
    println!("  oracle sample (Items, first 20 by name):");
    for s in oracle.iter().take(20) {
        let sp = &s.sprite;
        println!(
            "    {} rect {:?} border {:?} pivot {:?} ppu {} packed {:?} settings {}",
            sp.name,
            sp.rect,
            sp.border,
            sp.pivot,
            sp.pixels_to_units,
            s.packed.texture_rect,
            s.packed.settings_raw
        );
    }

    // The Items pages' decoded pixels, for the oracle's CRCs.
    if let Some(a) = sprites.atlases.get("Items") {
        let mut pages: Vec<&ObjectRef> = a.sprites.values().map(|s| &s.page).collect();
        pages.sort_by_key(|p| p.path_id);
        pages.dedup_by_key(|p| p.path_id);
        for p in pages {
            let t = assets.texture(p)?;
            let rgba = t.texture.decode_rgba(&t.data).map_err(|e| e.to_string())?;
            println!(
                "  page {} {} {}x{} format {} rgba crc {:08x}",
                p.path_id,
                t.texture.name,
                t.texture.width,
                t.texture.height,
                t.texture.format,
                crate::texture::crc32(&rgba)
            );
        }
    }
    if write_icons {
        let dir = std::path::Path::new("out/ui/icons");
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut written = 0;
        for (_, name) in &items {
            let Some(s) = sprites.item_icon(name) else {
                continue;
            };
            let (w, h, px) = assets.sprite_pixels(s)?;
            let path = dir.join(format!("{name}.png"));
            std::fs::write(&path, crate::png::encode_rgba(w, h, &px))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            written += 1;
        }
        println!("  wrote {written} icons to {}", dir.display());
    }
    println!(
        "{} checks differ ({:.1} s)",
        checks.failed,
        start.elapsed().as_secs_f64()
    );
    Ok(if checks.failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
